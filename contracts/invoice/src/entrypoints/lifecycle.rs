//! Invoice lifecycle entrypoints: creation, fee configuration and the shared
//! invoice-write core reused by the recurring-template keeper.

use crate::events;
use crate::invoice::{DataKey, Invoice, InvoiceError, InvoiceStatus, MaybeBytes};
use crate::validation::{
    require_admin, require_not_paused, require_positive_amount, require_usdc_precision,
};
use soroban_sdk::{Address, Env};

/// Largest late fee the contract accepts: 10_000 bps = 100% of the net amount.
pub const MAX_LATE_FEE_BPS: u32 = 10_000;

/// Validates the invoice parameters shared by `create_invoice` and the keeper.
fn validate_invoice_params(
    amount_usdc: i128,
    gross_usdc: i128,
    expires_in_seconds: u64,
) -> Result<(), InvoiceError> {
    require_positive_amount(amount_usdc, gross_usdc)?;
    require_usdc_precision(amount_usdc, gross_usdc)?;
    if expires_in_seconds == 0 {
        return Err(InvoiceError::ZeroDuration);
    }
    Ok(())
}

/// Writes a new invoice and returns its id. Performs no authorization: callers
/// either authenticate the merchant themselves (`create_invoice`) or are the
/// permissionless keeper acting on an already-authorized template.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_invoice_record(
    env: &Env,
    merchant: &Address,
    amount_usdc: i128,
    gross_usdc: i128,
    expires_in_seconds: u64,
    metadata_hash: MaybeBytes,
    payment_link_hash: MaybeBytes,
    merchant_nonce: u64,
) -> Result<u64, InvoiceError> {
    validate_invoice_params(amount_usdc, gross_usdc, expires_in_seconds)?;

    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::InvoiceCount)
        .unwrap_or(0);
    let id = count
        .checked_add(1)
        .ok_or(InvoiceError::InvoiceIdOverflow)?;
    let expires_at = env
        .ledger()
        .timestamp()
        .checked_add(expires_in_seconds)
        .ok_or(InvoiceError::ExpiryOverflow)?;
    let invoice = Invoice {
        id,
        merchant: merchant.clone(),
        amount_usdc,
        gross_usdc,
        status: InvoiceStatus::Pending,
        expires_at,
        paid_at: None,
        payer: crate::invoice::MaybeAddress::None,
        metadata_hash,
        payment_link_hash,
        merchant_nonce,
    };

    env.storage()
        .persistent()
        .set(&DataKey::Invoice(id), &invoice);
    env.storage().instance().set(&DataKey::InvoiceCount, &id);
    events::invoice_created(env, id, &invoice);
    Ok(id)
}

/// Creates an invoice with an optional merchant-supplied nonce for idempotency.
/// Pass `merchant_nonce = 0` to skip nonce enforcement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_invoice(
    env: &Env,
    merchant: Address,
    amount_usdc: i128,
    gross_usdc: i128,
    expires_in_seconds: u64,
    metadata_hash: MaybeBytes,
    payment_link_hash: MaybeBytes,
    merchant_nonce: u64,
) -> Result<u64, InvoiceError> {
    merchant.require_auth();
    require_not_paused(env)?;
    validate_invoice_params(amount_usdc, gross_usdc, expires_in_seconds)?;

    // Issue #58: reject a duplicate merchant nonce.
    if merchant_nonce != 0 {
        let nonce_key = DataKey::MerchantNonce(merchant.clone(), merchant_nonce);
        if env.storage().persistent().has(&nonce_key) {
            return Err(InvoiceError::DuplicateNonce);
        }
        env.storage().persistent().set(&nonce_key, &true);
    }

    create_invoice_record(
        env,
        &merchant,
        amount_usdc,
        gross_usdc,
        expires_in_seconds,
        metadata_hash,
        payment_link_hash,
        merchant_nonce,
    )
}

/// Sets the late fee, in basis points, charged on overdue balances (admin-only).
/// Errors: `Unauthorized`, `InvalidLateFee`.
/// Emits: `late_fee_updated`.
pub(crate) fn set_late_fee_bps(env: &Env, admin: &Address, bps: u32) -> Result<(), InvoiceError> {
    require_admin(env, admin)?;
    if bps > MAX_LATE_FEE_BPS {
        return Err(InvoiceError::InvalidLateFee);
    }
    let old_bps: u32 = env
        .storage()
        .instance()
        .get(&DataKey::LateFeeBps)
        .unwrap_or(0);
    env.storage().instance().set(&DataKey::LateFeeBps, &bps);
    events::late_fee_updated(env, old_bps, bps);
    Ok(())
}

/// Returns the configured late fee in basis points (0 if never set).
pub(crate) fn get_late_fee_bps(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::LateFeeBps)
        .unwrap_or(0)
}
