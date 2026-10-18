use crate::invoice::{DataKey, InvoiceError, MaybeBytes, MAX_HASH_BYTES, MAX_LATE_FEE_BPS, USDC_FACTOR};
use soroban_sdk::{Address, Env};

/// Maximum allowed expiry duration: 5 years in seconds.
pub const MAX_EXPIRY_SECONDS: u64 = 5 * 365 * 24 * 60 * 60;

pub fn require_not_paused(env: &Env) -> Result<(), InvoiceError> {
    let paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    if paused {
        return Err(InvoiceError::ContractPaused);
    }
    Ok(())
}

pub fn require_admin(env: &Env, admin: &Address) -> Result<(), InvoiceError> {
    admin.require_auth();
    // An uninitialized contract has no admin, so it fails closed as Unauthorized.
    let stored: Option<Address> = env.storage().instance().get(&DataKey::Admin);
    if stored.as_ref() != Some(admin) {
        return Err(InvoiceError::Unauthorized);
    }
    Ok(())
}

pub fn require_positive_amount(amount_usdc: i128, gross_usdc: i128) -> Result<(), InvoiceError> {
    if amount_usdc <= 0 || gross_usdc < amount_usdc {
        return Err(InvoiceError::InvalidAmount);
    }
    Ok(())
}

/// Reject a late fee (the difference `gross_usdc − amount_usdc`) that exceeds
/// [`MAX_LATE_FEE_BPS`] of `amount_usdc`.
///
/// `MAX_LATE_FEE_BPS` is the single source of truth for the cap; it is defined
/// in `invoice.rs` and imported here rather than redeclared, so the storage
/// model and the validation layer can never disagree about the limit.
///
/// Prerequisite: `amount_usdc > 0` and `gross_usdc >= amount_usdc` (enforced
/// by `require_positive_amount`).
pub fn require_late_fee_within_cap(amount_usdc: i128, gross_usdc: i128) -> Result<(), InvoiceError> {
    let late_fee = gross_usdc - amount_usdc;
    // late_fee * 10_000 <= amount_usdc * MAX_LATE_FEE_BPS
    // Rearranged to avoid overflow: late_fee / amount_usdc <= MAX_LATE_FEE_BPS / 10_000.
    // We cross-multiply instead: late_fee * 10_000 <= amount_usdc * MAX_LATE_FEE_BPS.
    // Both sides fit in i128 for any reasonable invoice amount.
    if late_fee * 10_000 > amount_usdc * i128::from(MAX_LATE_FEE_BPS) {
        return Err(InvoiceError::InvalidAmount);
    }
    Ok(())
}

/// Reject amounts below the minimum USDC unit (1 USDC = USDC_FACTOR stroops).
/// This guards against off-by-factor errors (e.g., passing dollar cents instead of stroops).
pub fn require_usdc_precision(amount_usdc: i128, gross_usdc: i128) -> Result<(), InvoiceError> {
    if amount_usdc < USDC_FACTOR || gross_usdc < USDC_FACTOR {
        return Err(InvoiceError::AmountPrecision);
    }
    Ok(())
}

/// Reject expires_in_seconds values that exceed MAX_EXPIRY_SECONDS.
pub fn require_expiry_not_too_long(expires_in_seconds: u64) -> Result<(), InvoiceError> {
    if expires_in_seconds > MAX_EXPIRY_SECONDS {
        return Err(InvoiceError::ExpiryTooLong);
    }
    Ok(())
}

/// Reject optional invoice hash fields that exceed the storage/cost cap.
pub fn require_hash_not_too_long(hash: &MaybeBytes) -> Result<(), InvoiceError> {
    if let MaybeBytes::Some(bytes) = hash {
        if bytes.len() > MAX_HASH_BYTES {
            return Err(InvoiceError::HashTooLong);
        }
    }
    Ok(())
}

/// Reject a payment_link_hash that is provided but not exactly 32 bytes.
pub fn require_valid_payment_link_hash(hash: &MaybeBytes) -> Result<(), InvoiceError> {
    if let MaybeBytes::Some(bytes) = hash {
        if bytes.len() != 32 {
            return Err(InvoiceError::InvalidPaymentLinkHash);
        }
    }
    Ok(())
}
