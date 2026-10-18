//! Invoice template entrypoints.
//!
//! Templates allow merchants to define reusable invoice blueprints and generate
//! individual invoices from them without repeating common fields each time.
//! Each template is merchant-owned; only the creating merchant may disable it.

use crate::events;
use crate::validation::{
    require_hash_not_too_long, require_not_paused, require_positive_amount, require_usdc_precision,
    require_expiry_not_too_long,
};
use crate::{
    pending_index_add, DataKey, Invoice, InvoiceContract, InvoiceContractArgs,
    InvoiceContractClient, InvoiceError, InvoiceStatus, InvoiceTemplate, MaybeAddress, MaybeBytes,
};
use soroban_sdk::{contractimpl, Address, Env, Vec};

#[contractimpl]
impl InvoiceContract {
    // --- #694: invoice template entrypoints ---

    /// Create a new reusable invoice template.
    ///
    /// Only the merchant themselves may create a template under their address.
    /// Returns the assigned template ID.
    #[allow(clippy::too_many_arguments)]
    pub fn create_template(
        env: Env,
        merchant: Address,
        amount_usdc: i128,
        gross_usdc: i128,
        duration_seconds: u64,
        metadata_hash: MaybeBytes,
        token_address: MaybeAddress,
    ) -> Result<u64, InvoiceError> {
        merchant.require_auth();
        require_not_paused(&env)?;
        require_positive_amount(amount_usdc, gross_usdc)?;
        require_usdc_precision(amount_usdc, gross_usdc)?;
        require_hash_not_too_long(&metadata_hash)?;
        if duration_seconds == 0 {
            return Err(InvoiceError::ZeroDuration);
        }
        require_expiry_not_too_long(duration_seconds)?;

        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::TemplateCount)
            .unwrap_or(0);
        let id = count
            .checked_add(1)
            .ok_or(InvoiceError::InvoiceCountOverflow)?;

        let template = InvoiceTemplate {
            id,
            merchant: merchant.clone(),
            amount_usdc,
            gross_usdc,
            duration_seconds,
            metadata_hash,
            token_address,
            enabled: true,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Template(id), &template);
        env.storage().instance().set(&DataKey::TemplateCount, &id);

        // Update merchant-specific template index
        let merchant_count_key = DataKey::MerchantTemplateCount(merchant.clone());
        let merchant_count: u64 = env
            .storage()
            .persistent()
            .get(&merchant_count_key)
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::MerchantTemplateIndex(merchant.clone(), merchant_count),
            &id,
        );
        env.storage()
            .persistent()
            .set(&merchant_count_key, &(merchant_count + 1));

        events::template_created(&env, id, &merchant, amount_usdc, gross_usdc, duration_seconds);
        Ok(id)
    }

    /// Generate a new invoice from an existing enabled template.
    ///
    /// Uses the template's amount, gross, duration, metadata hash, and token
    /// address to populate the invoice. The merchant who owns the template must
    /// authorise this call. Returns the newly created invoice ID.
    pub fn generate_from_template(
        env: Env,
        merchant: Address,
        template_id: u64,
        merchant_nonce: u64,
    ) -> Result<u64, InvoiceError> {
        merchant.require_auth();
        require_not_paused(&env)?;

        let template: InvoiceTemplate = env
            .storage()
            .persistent()
            .get(&DataKey::Template(template_id))
            .ok_or(InvoiceError::NotFound)?;

        if template.merchant != merchant {
            return Err(InvoiceError::Unauthorized);
        }
        if !template.enabled {
            return Err(InvoiceError::NotPending);
        }

        // Reject duplicate nonce if provided
        if merchant_nonce != 0 {
            let nonce_key = DataKey::MerchantNonce(merchant.clone(), merchant_nonce);
            if env.storage().persistent().has(&nonce_key) {
                return Err(InvoiceError::DuplicateNonce);
            }
        }

        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::InvoiceCount)
            .unwrap_or(0);
        let id = count
            .checked_add(1)
            .ok_or(InvoiceError::InvoiceCountOverflow)?;
        let expires_at = env
            .ledger()
            .timestamp()
            .checked_add(template.duration_seconds)
            .ok_or(InvoiceError::ExpiryOverflow)?;

        if merchant_nonce != 0 {
            env.storage().persistent().set(
                &DataKey::MerchantNonce(merchant.clone(), merchant_nonce),
                &true,
            );
        }

        let invoice = Invoice {
            id,
            merchant: merchant.clone(),
            amount_usdc: template.amount_usdc,
            gross_usdc: template.gross_usdc,
            status: InvoiceStatus::Pending,
            expires_at,
            paid_at: None,
            payer: MaybeAddress::None,
            metadata_hash: template.metadata_hash.clone(),
            payment_link_hash: MaybeBytes::None,
            merchant_nonce,
            token_address: template.token_address.clone(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        env.storage().instance().set(&DataKey::InvoiceCount, &id);

        let merchant_count_key = DataKey::MerchantInvoiceCount(merchant.clone());
        let merchant_count: u64 = env
            .storage()
            .persistent()
            .get(&merchant_count_key)
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::MerchantInvoiceIndex(merchant.clone(), merchant_count),
            &id,
        );
        env.storage()
            .persistent()
            .set(&merchant_count_key, &(merchant_count + 1));

        pending_index_add(&env, id);
        events::invoice_created(&env, id, &invoice);
        events::template_generated(&env, template_id, id, &merchant);
        Ok(id)
    }

    /// Disable an invoice template. Only the owning merchant may disable it.
    ///
    /// Disabled templates can no longer be used to generate invoices.
    /// Invoices already generated from the template are unaffected.
    pub fn disable_template(
        env: Env,
        merchant: Address,
        template_id: u64,
    ) -> Result<(), InvoiceError> {
        merchant.require_auth();
        require_not_paused(&env)?;

        let mut template: InvoiceTemplate = env
            .storage()
            .persistent()
            .get(&DataKey::Template(template_id))
            .ok_or(InvoiceError::NotFound)?;

        if template.merchant != merchant {
            return Err(InvoiceError::Unauthorized);
        }

        template.enabled = false;
        env.storage()
            .persistent()
            .set(&DataKey::Template(template_id), &template);

        events::template_disabled(&env, template_id, &merchant);
        Ok(())
    }

    /// Retrieve a template by its ID.
    pub fn get_template(env: Env, template_id: u64) -> Result<InvoiceTemplate, InvoiceError> {
        env.storage()
            .persistent()
            .get(&DataKey::Template(template_id))
            .ok_or(InvoiceError::NotFound)
    }

    /// Return the total number of templates created.
    pub fn get_template_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::TemplateCount)
            .unwrap_or(0u64)
    }

    /// Return a page of template IDs for `merchant`.
    /// `start` is a zero-based offset; `limit` caps the returned slice.
    pub fn get_templates_by_merchant(
        env: Env,
        merchant: Address,
        start: u32,
        limit: u32,
    ) -> Vec<u64> {
        let total: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::MerchantTemplateCount(merchant.clone()))
            .unwrap_or(0);
        let start = u64::from(start).min(total);
        let end = start.saturating_add(u64::from(limit)).min(total);
        let mut page = Vec::new(&env);
        for i in start..end {
            if let Some(id) = env
                .storage()
                .persistent()
                .get::<DataKey, u64>(&DataKey::MerchantTemplateIndex(merchant.clone(), i))
            {
                page.push_back(id);
            }
        }
        page
    }
}
