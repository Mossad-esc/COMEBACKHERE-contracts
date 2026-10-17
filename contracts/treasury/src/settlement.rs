use crate::multisig::DataKey;
use soroban_sdk::{Address, Env};

/// Returns the approval weight assigned to `signer`, or `0` if not registered.
pub fn signer_weight(env: &Env, signer: &Address) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::Signer(signer.clone()))
        .unwrap_or(0)
}

/// Requires `signer` to authenticate and have a non-zero weight in the signer registry.
/// Panics: `UnauthorizedSigner`.
pub fn require_authorized_signer(env: &Env, signer: &Address) {
    signer.require_auth();
    if signer_weight(env, signer) == 0 {
        panic!("UnauthorizedSigner");
    }
}

/// Stamps `signer` as active at the current ledger timestamp under
/// `DataKey::SignerLastActive`. Called on every approval path so the signer
/// registry can report who has gone quiet between rotations.
pub fn record_approval(env: &Env, signer: &Address) {
    env.storage().instance().set(
        &DataKey::SignerLastActive(signer.clone()),
        &env.ledger().timestamp(),
    );
}

/// Returns the ledger timestamp of `signer`'s most recent approval, or `None`
/// if the signer has never been recorded as active.
pub fn signer_last_active(env: &Env, signer: &Address) -> Option<u64> {
    env.storage()
        .instance()
        .get(&DataKey::SignerLastActive(signer.clone()))
}
