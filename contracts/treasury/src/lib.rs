#![no_std]

pub use multisig::{
    DataKey, Dispute, DisputeStatus, MaybeAddress, RotationStatus, Settlement,
    SettlementHoldReason, SettlementStatus, SignerChangeKind, SignerChangeProposal,
    SignerChangeStatus, SignerRotationProposal, TreasuryError,
};

use soroban_sdk::{contract, contractimpl, Address, Env, Symbol, Vec};

mod admin;
mod deposits;
mod disputes;
mod holds;
mod settlements;
mod signers;
mod timelock;

#[contract]
pub struct TreasuryContract;

#[contractimpl]
impl TreasuryContract {
    /// Initialises the treasury with `admin` as owner and `threshold` as the multisig approval
    /// weight required to execute settlements. Accepts an initial `signers` list of `(Address, u32)`
    /// pairs to bootstrap the full signer set in a single transaction.
    /// Errors: `AlreadyInitialized`, `ZeroThreshold`.
    /// Emits: `treasury_initialized`.
    pub fn initialize(
        env: Env,
        admin: Address,
        threshold: u32,
        signers: Vec<(Address, u32)>,
    ) -> Result<(), TreasuryError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(TreasuryError::AlreadyInitialized);
        }
        if threshold == 0 {
            return Err(TreasuryError::ZeroThreshold);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::Threshold, &threshold);
        env.storage()
            .instance()
            .set(&DataKey::SettlementCount, &0u64);
        env.storage().instance().set(&DataKey::Paused, &false);
        env.storage().instance().set(&DataKey::DisputeCount, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::SettlementExpirySecs, &(7u64 * 24 * 60 * 60));
        env.storage()
            .instance()
            .set(&DataKey::Signer(admin.clone()), &1u32);
        let mut signer_list = Vec::new(&env);
        signer_list.push_back(admin.clone());
        for (signer, weight) in signers.iter() {
            env.storage()
                .instance()
                .set(&DataKey::Signer(signer.clone()), &weight);
            if weight > 0 && !signer_list.contains(&signer) {
                signer_list.push_back(signer.clone());
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::SignerList, &signer_list);
        env.events()
            .publish((Symbol::new(&env, "treasury_initialized"),), admin);
        Ok(())
    }

    /// Read-only view of how far a partial settlement has progressed (#738).
    /// Returns `(approved_total, executed_total, remaining)` where `approved_total` is the
    /// internally tracked `PartialApprovedTotal`, `executed_total` is the amount already
    /// executed, and `remaining` is `approved_total - executed_total` (saturating at zero).
    /// Errors: `SettlementNotFound`.
    pub fn get_partial_progress(env: Env, settlement_id: u64) -> Result<(i128, i128, i128), TreasuryError> {
        settlements::get_partial_progress(&env, settlement_id)
    }

    /// Registers or updates a signer's approval weight. Rejects a weight of `0` with
    /// `ZeroSignerWeight` — callers must use `remove_signer` to drop a signer instead of
    /// leaving a zero-weight entry in `SignerList` (#739).
    /// Errors: `Unauthorized`, `ZeroSignerWeight`.
    pub fn set_signer(env: Env, signer: Address, weight: u32) -> Result<(), TreasuryError> {
        signers::set_signer(&env, signer, weight)
    }

    /// Sets a human-readable label for `signer`. The label is capped at
    /// `MAX_SIGNER_LABEL_LEN` bytes to bound storage costs (#740).
    /// Errors: `Unauthorized`, `SignerLabelTooLong`.
    pub fn set_signer_label(env: Env, signer: Address, label: String) -> Result<(), TreasuryError> {
        signers::set_signer_label(&env, signer, label)
    }
}

/// Maximum number of tokens allowed in the allowlist to prevent unbounded storage growth.
pub(crate) const MAX_ALLOWED_TOKENS: u32 = 20;

/// Maximum length, in bytes, of a signer label accepted by `set_signer_label`.
/// Bounds storage costs by rejecting unbounded caller-supplied strings (#740).
pub(crate) const MAX_SIGNER_LABEL_LEN: u32 = 64;

pub(crate) fn require_admin(env: &Env, admin: &Address) {
    admin.require_auth();
    // An uninitialized contract has no admin, so it fails closed as Unauthorized.
    let stored: Option<Address> = env.storage().instance().get(&DataKey::Admin);
    if stored.as_ref() != Some(admin) {
        soroban_sdk::panic_with_error!(env, TreasuryError::Unauthorized);
    }
}

pub(crate) fn require_not_paused(env: &Env) {
    let paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::Paused)
        .unwrap_or(false);
    if paused {
        soroban_sdk::panic_with_error!(env, TreasuryError::ContractPaused);
    }
}

/// Writes `settlement` to storage and keeps `DataKey::PendingSettlementIndex` in sync
/// with its `status` (#572). This is the *only* place a `Settlement` should be persisted
/// in this crate — every write site (propose, approve, execute, cancel, expire, hold,
/// dispute-driven hold/release, force-cancel) goes through here so the index can never
/// drift from what `settlement.status == Pending` actually says, without having to
/// duplicate that add/remove bookkeeping at each call site individually.
pub(crate) fn write_settlement(env: &Env, id: u64, settlement: &Settlement) {
    env.storage()
        .persistent()
        .set(&DataKey::Settlement(id), settlement);

    let mut index: Vec<u64> = env
        .storage()
        .instance()
        .get(&DataKey::PendingSettlementIndex)
        .unwrap_or_else(|| Vec::new(env));
    let is_pending = settlement.status == SettlementStatus::Pending;
    let already_indexed = index.contains(id);

    if is_pending && !already_indexed {
        index.push_back(id);
        env.storage()
            .instance()
            .set(&DataKey::PendingSettlementIndex, &index);
    } else if !is_pending && already_indexed {
        let mut updated = Vec::new(env);
        for existing in index.iter() {
            if existing != id {
                updated.push_back(existing);
            }
        }
        env.storage()
            .instance()
            .set(&DataKey::PendingSettlementIndex, &updated);
    }
}

#[cfg(test)]
extern crate std;
