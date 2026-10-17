#![no_std]

mod entrypoints;
mod events;
mod invoice;
mod template;
mod validation;

pub use invoice::{DataKey, Invoice, InvoiceError, InvoiceStatus, MaybeAddress, MaybeBytes};
pub use template::{InvoiceTemplate, RecurrenceInterval, MAX_BATCH_SIZE, MAX_PAGE_LIMIT};

use entrypoints::lifecycle::{create_invoice, get_late_fee_bps, set_late_fee_bps};
use entrypoints::template::{
    create_template, generate_due_invoices, get_invoice_template, list_templates_by_merchant,
    set_template_active,
};
use soroban_sdk::{contract, contractimpl, Address, Env, Vec};
use validation::{require_admin, require_not_paused};

#[contract]
pub struct InvoiceContract;

#[contractimpl]
impl InvoiceContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), InvoiceError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(InvoiceError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::InvoiceCount, &0u64);
        env.storage().instance().set(&DataKey::Paused, &false);
        Ok(())
    }

    // --- #55: configurable grace window ---

    /// Set the grace window (seconds) added to expires_at when checking payment validity.
    /// Allows a short buffer after quote expiry for in-flight payments.
    pub fn set_grace_window(env: Env, admin: Address, seconds: u64) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::GraceWindow, &seconds);
        Ok(())
    }

    /// Return the current grace window in seconds (0 if not set).
    pub fn get_grace_window(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::GraceWindow)
            .unwrap_or(0u64)
    }

    // --- #58: merchant invoice nonce ---

    /// Create an invoice with an optional merchant-supplied nonce for idempotency.
    /// Pass `merchant_nonce = 0` to skip nonce enforcement.
    /// A non-zero nonce that has already been used for this merchant is rejected.
    #[allow(clippy::too_many_arguments)]
    pub fn create_invoice(
        env: Env,
        merchant: Address,
        amount_usdc: i128,
        gross_usdc: i128,
        expires_in_seconds: u64,
        metadata_hash: MaybeBytes,
        payment_link_hash: MaybeBytes,
        merchant_nonce: u64,
    ) -> Result<u64, InvoiceError> {
        create_invoice(
            &env,
            merchant,
            amount_usdc,
            gross_usdc,
            expires_in_seconds,
            metadata_hash,
            payment_link_hash,
            merchant_nonce,
        )
    }

    pub fn mark_paid(
        env: Env,
        admin: Address,
        id: u64,
        payer: Address,
    ) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        require_not_paused(&env)?;

        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;

        if invoice.status != InvoiceStatus::Pending {
            return Err(InvoiceError::NotPending);
        }

        // #55: apply grace window — payment is valid up to expires_at + grace_window
        let grace: u64 = env
            .storage()
            .instance()
            .get(&DataKey::GraceWindow)
            .unwrap_or(0u64);
        let effective_deadline = invoice
            .expires_at
            .checked_add(grace)
            .unwrap_or(invoice.expires_at);
        if env.ledger().timestamp() >= effective_deadline {
            return Err(InvoiceError::Expired);
        }

        invoice.status = InvoiceStatus::Paid;
        invoice.paid_at = Some(env.ledger().timestamp());
        invoice.payer = MaybeAddress::Some(payer);
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        events::invoice_paid(&env, id, &invoice);
        Ok(())
    }

    // --- #56: escrow release entrypoint ---

    /// Release escrow for a paid invoice. Admin-only. Transitions Paid → Released.
    pub fn release_escrow(env: Env, admin: Address, id: u64) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        require_not_paused(&env)?;

        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;

        if invoice.status != InvoiceStatus::Paid {
            return Err(InvoiceError::NotPaid);
        }

        invoice.status = InvoiceStatus::Released;
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        events::escrow_released(&env, id, &invoice);
        Ok(())
    }

    pub fn get_invoice(env: Env, id: u64) -> Result<Invoice, InvoiceError> {
        env.storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)
    }

    pub fn get_invoice_status(env: Env, id: u64) -> Result<InvoiceStatus, InvoiceError> {
        let invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;
        Ok(invoice.status)
    }

    // Issue #49: merchant or admin may cancel a pending invoice
    pub fn cancel_invoice(env: Env, caller: Address, id: u64) -> Result<(), InvoiceError> {
        caller.require_auth();
        require_not_paused(&env)?;

        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;

        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if caller != invoice.merchant && caller != admin {
            return Err(InvoiceError::Unauthorized);
        }
        if invoice.status != InvoiceStatus::Pending {
            return Err(InvoiceError::NotPending);
        }

        invoice.status = InvoiceStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        events::invoice_cancelled(&env, id, &invoice);
        Ok(())
    }

    pub fn batch_expire(env: Env, admin: Address, ids: Vec<u64>) -> Result<u32, InvoiceError> {
        require_admin(&env, &admin)?;
        let now = env.ledger().timestamp();
        let mut expired_count: u32 = 0;
        for id in ids.iter() {
            let key = DataKey::Invoice(id);
            if let Some(mut invoice) = env.storage().persistent().get::<DataKey, Invoice>(&key) {
                if invoice.status == InvoiceStatus::Pending && now >= invoice.expires_at {
                    invoice.status = InvoiceStatus::Expired;
                    env.storage().persistent().set(&key, &invoice);
                    events::invoice_expired(&env, id, &invoice);
                    expired_count += 1;
                }
            }
        }
        Ok(expired_count)
    }

    // payer may request a refund on a paid invoice (escrow dispute)
    pub fn request_refund(env: Env, payer: Address, id: u64) -> Result<(), InvoiceError> {
        payer.require_auth();
        require_not_paused(&env)?;

        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;

        if invoice.status != InvoiceStatus::Paid {
            return Err(InvoiceError::NotPaid);
        }
        if invoice.payer != MaybeAddress::Some(payer.clone()) {
            return Err(InvoiceError::Unauthorized);
        }

        invoice.status = InvoiceStatus::RefundRequested;
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        events::invoice_refund_requested(&env, id, &invoice);
        Ok(())
    }

    pub fn pause(env: Env, admin: Address) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Paused, &true);
        events::contract_paused(&env, &admin);
        Ok(())
    }

    pub fn unpause(env: Env, admin: Address) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Paused, &false);
        events::contract_unpaused(&env, &admin);
        Ok(())
    }

    // --- recurring invoice templates (#698, #699) ---

    /// Register a recurring invoice template for `merchant` and return its id.
    /// The first run is scheduled one full interval after creation, so a new
    /// template is never due immediately.
    /// Errors: `ContractPaused`, `InvalidAmount`, `AmountPrecision`, `ZeroDuration`,
    ///         `ExpiryOverflow`, `InvoiceIdOverflow`.
    /// Emits: `template_created`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_template(
        env: Env,
        merchant: Address,
        amount_usdc: i128,
        gross_usdc: i128,
        interval: RecurrenceInterval,
        expires_in_seconds: u64,
        metadata_hash: MaybeBytes,
        payment_link_hash: MaybeBytes,
    ) -> Result<u64, InvoiceError> {
        create_template(
            &env,
            merchant,
            amount_usdc,
            gross_usdc,
            interval,
            expires_in_seconds,
            metadata_hash,
            payment_link_hash,
        )
    }

    /// Return the recurring invoice template registered under `id`.
    /// Errors: `NotFound`.
    pub fn get_invoice_template(env: Env, id: u64) -> Result<InvoiceTemplate, InvoiceError> {
        get_invoice_template(&env, id)
    }

    /// Return one page of `merchant`'s templates in creation order, skipping the
    /// first `start` entries. `limit` is capped at `MAX_PAGE_LIMIT`.
    pub fn list_templates_by_merchant(
        env: Env,
        merchant: Address,
        start: u64,
        limit: u64,
    ) -> Vec<InvoiceTemplate> {
        list_templates_by_merchant(&env, merchant, start, limit)
    }

    /// Activate or deactivate a template (merchant or admin).
    /// Errors: `NotFound`, `Unauthorized`.
    /// Emits: `template_activated`.
    pub fn set_template_active(
        env: Env,
        caller: Address,
        id: u64,
        active: bool,
    ) -> Result<(), InvoiceError> {
        set_template_active(&env, &caller, id, active)
    }

    /// Permissionless keeper call: create one invoice for each due template in
    /// `template_ids` and advance its schedule. Unknown, inactive and
    /// not-yet-due templates are skipped, and a period can never be invoiced
    /// twice. At most `MAX_BATCH_SIZE` ids are accepted.
    /// Errors: `BatchTooLarge`, `ContractPaused`.
    /// Emits: `template_generated` per generated invoice.
    pub fn generate_due_invoices(env: Env, template_ids: Vec<u64>) -> Result<u32, InvoiceError> {
        generate_due_invoices(&env, template_ids)
    }

    // --- late fee (#704) ---

    /// Set the late fee charged on overdue balances, in basis points. Values
    /// above `MAX_LATE_FEE_BPS` (10_000 = 100%) are rejected.
    /// Errors: `Unauthorized`, `InvalidLateFee`.
    /// Emits: `late_fee_updated` carrying the old and new bps.
    pub fn set_late_fee_bps(env: Env, admin: Address, bps: u32) -> Result<(), InvoiceError> {
        set_late_fee_bps(&env, &admin, bps)
    }

    /// Return the configured late fee in basis points (0 if never set).
    pub fn get_late_fee_bps(env: Env) -> u32 {
        get_late_fee_bps(&env)
    }
}

#[cfg(test)]
extern crate std;
