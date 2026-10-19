#![no_std]

pub use admin::EmergencyWithdrawProposal;
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
mod settlement_approvals;
mod settlement_execution;
mod settlement_proposals;
mod settlement_queries;
mod signers;
mod timelock;
mod tokens;

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

    /// Proposes `new_admin` as the next treasury admin (two-step handover, #741).
    /// The current admin must authorise the call. The proposal stays pending until
    /// `accept_admin` is called by `new_admin`, or it is cancelled/expires.
    /// Errors: `Unauthorized`, `AdminTransferPending`.
    /// Emits: `admin_transfer_proposed`.
    pub fn transfer_admin(
        env: Env,
        new_admin: Address,
    ) -> Result<(), TreasuryError> {
        admin::transfer_admin(&env, &new_admin)
    }

    /// Finalises a pending admin handover. Must be called by the proposed `new_admin`.
    /// Errors: `Unauthorized`, `NoPendingAdminTransfer`, `AdminTransferExpired`.
    /// Emits: `admin_transferred`.
    pub fn accept_admin(env: Env) -> Result<(), TreasuryError> {
        admin::accept_admin(&env)
    }

    /// Cancels a pending admin handover. Must be called by the current admin.
    /// Errors: `Unauthorized`, `NoPendingAdminTransfer`.
    /// Emits: `admin_transfer_cancelled`.
    pub fn cancel_admin_transfer(env: Env) -> Result<(), TreasuryError> {
        admin::cancel_admin_transfer(&env)
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

/// Maximum number of bytes allowed in a dispute resolution memo (#744).
pub(crate) const MAX_DISPUTE_MEMO_LEN: u32 = 256;
/// Maximum length, in bytes, of a signer label accepted by `set_signer_label`.
/// Bounds storage costs by rejecting unbounded caller-supplied strings (#740).
pub(crate) const MAX_SIGNER_LABEL_LEN: u32 = 64;
/// Default TTL (in ledgers) for a deposit reference id before it can be reused (#747).
pub(crate) const DEPOSIT_REF_TTL_LEDGERS: u32 = 30 * 24 * 60 * 60 / 5;

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

/// Enforces the recipient's compliance tier transaction limit before a settlement
/// transfers funds (#757). Calls `is_allowed_for_amount` on the configured
/// `compliance-client` contract; if the recipient is not allowed for `amount` the
/// settlement execution fails closed with `TreasuryError::ComplianceRejected`.
///
/// When no compliance client is configured the check is skipped so existing
/// deployments without compliance keep working.
pub(crate) fn require_compliance_allowed(
    env: &Env,
    recipient: &Address,
    amount: i128,
) -> Result<(), TreasuryError> {
    let client: Option<Address> = env
        .storage()
        .instance()
        .get(&DataKey::ComplianceClient);
    let Some(compliance) = client else {
        return Ok(());
    };
    let allowed: bool = env.invoke_contract(
        &compliance,
        &Symbol::new(env, "is_allowed_for_amount"),
        soroban_sdk::vec![env, recipient.to_val(), amount.into()],
    );
    if !allowed {
        return Err(TreasuryError::ComplianceRejected);
    }
    Ok(())
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

/// Clears any hold-related storage for `id` (#745). Called from every terminal
/// settlement transition (execute, cancel, expire, force-cancel) so a held
/// settlement never leaves `HoldExpiry` / `HoldReason` entries behind once it
/// is no longer live. Idempotent: safe to call when no hold was ever set.
pub(crate) fn clear_hold_state(env: &Env, id: u64) {
    env.storage().persistent().remove(&DataKey::HoldExpiry(id));
    env.storage().persistent().remove(&DataKey::HoldReason(id));
}

/// Rejects a deposit whose reference id was already seen within the TTL window
/// (#747). Records `ref_id` with a TTL so storage stays bounded and the id can
/// be reused once the entry expires. Returns `Err(TreasuryError::DuplicateReferenceId)`
/// on a duplicate.
pub(crate) fn check_and_record_deposit_ref(
    env: &Env,
    ref_id: &Symbol,
) -> Result<(), TreasuryError> {
    let key = DataKey::DepositRef(ref_id.clone());
    if env.storage().persistent().has(&key) {
        return Err(TreasuryError::DuplicateReferenceId);
    }
    env.storage().persistent().set(&key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&key, DEPOSIT_REF_TTL_LEDGERS, DEPOSIT_REF_TTL_LEDGERS);
    Ok(())
}

#[cfg(test)]
extern crate std;
