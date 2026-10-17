use crate::events;
use crate::events::{InvoiceExpiryExtendedEvent, MerchantMaxOpenInvoicesUpdatedEvent};
use crate::validation::require_admin;
use crate::{DataKey, Invoice, InvoiceContract, InvoiceError, InvoiceStatus};
use soroban_sdk::{contractimpl, Address, Env};

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

    // --- #15: two-step admin transfer ---

    /// Initiate admin transfer. Current admin nominates `new_admin`.
    pub fn transfer_admin(
        env: Env,
        admin: Address,
        new_admin: Address,
    ) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        Ok(())
    }

    /// Complete admin transfer. Must be called by the pending admin.
    pub fn accept_admin(env: Env, new_admin: Address) -> Result<(), InvoiceError> {
        new_admin.require_auth();
        let pending: Address = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(InvoiceError::NoPendingAdmin)?;
        if pending != new_admin {
            return Err(InvoiceError::Unauthorized);
        }
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
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

    /// Extend the expiry of a pending invoice.
    pub fn extend_expiry(
        env: Env,
        admin: Address,
        id: u64,
        additional_seconds: u64,
    ) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;
        if invoice.status != InvoiceStatus::Pending {
            return Err(InvoiceError::NotPending);
        }

        let old_expires_at = invoice.expires_at;
        invoice.expires_at = old_expires_at
            .checked_add(additional_seconds)
            .filter(|new_expires_at| *new_expires_at > old_expires_at)
            .ok_or(InvoiceError::ExpiryOverflow)?;
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        events::invoice_expiry_extended(
            &env,
            &InvoiceExpiryExtendedEvent {
                id,
                old_expires_at,
                new_expires_at: invoice.expires_at,
            },
        );
        Ok(())
    }

    // --- #715: per-merchant open-invoice limit override ---

    /// Set the global cap on open (non-terminal) invoices per merchant.
    /// Merchants without a per-merchant override (see `set_merchant_max_open_invoices`)
    /// use this value.
    pub fn set_max_open_invoices(env: Env, admin: Address, limit: u64) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::MaxOpenInvoices, &limit);
        Ok(())
    }

    /// Return the current global open-invoice cap (0 if never set, meaning unlimited).
    pub fn get_max_open_invoices(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::MaxOpenInvoices)
            .unwrap_or(0u64)
    }

    /// Set a per-merchant override of the open-invoice cap. Pass `None` to clear
    /// the override, falling back to the global cap. Admin-only.
    pub fn set_merchant_max_open_invoices(
        env: Env,
        admin: Address,
        merchant: Address,
        limit: Option<u64>,
    ) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        let key = DataKey::MerchantMaxOpenInvoices(merchant.clone());
        match limit {
            Some(limit) => {
                env.storage().persistent().set(&key, &limit);
            }
            None => {
                env.storage().persistent().remove(&key);
            }
        }
        events::merchant_max_open_invoices_updated(
            &env,
            &MerchantMaxOpenInvoicesUpdatedEvent {
                merchant,
                new_limit: limit,
            },
        );
        Ok(())
    }

    /// Return the effective open-invoice cap for `merchant`: the per-merchant
    /// override if one is set, otherwise the global cap.
    pub fn get_max_open_invoices_for(env: Env, merchant: Address) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::MerchantMaxOpenInvoices(merchant))
            .unwrap_or_else(|| {
                env.storage()
                    .instance()
                    .get(&DataKey::MaxOpenInvoices)
                    .unwrap_or(0u64)
            })
    }
}
