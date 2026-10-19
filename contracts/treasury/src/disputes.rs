use crate::{
    require_admin, require_not_paused, write_settlement, DataKey, Dispute, DisputeStatus,
    Settlement, SettlementHoldReason, SettlementStatus, TreasuryContract, TreasuryContractArgs,
    TreasuryContractClient, TreasuryError,
};
use multisig::{meets_threshold, record_approval, require_authorized_signer};
use soroban_sdk::{contractimpl, token, Address, BytesN, Env, Symbol, Vec};

/// Basis-points denominator for `resolve_dispute_split`'s ratio (10_000 = 100.00%).
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Maximum length (in bytes) of the optional resolution `memo` accepted by
/// `resolve_dispute` and `resolve_dispute_split` (#744).
pub const MAX_RESOLUTION_MEMO_LEN: u32 = 256;

/// Validates an optional resolution memo against [`MAX_RESOLUTION_MEMO_LEN`].
fn validate_resolution_memo(memo: &Option<soroban_sdk::String>) -> Result<(), TreasuryError> {
    if let Some(m) = memo {
        if m.len() > MAX_RESOLUTION_MEMO_LEN {
            return Err(TreasuryError::InvalidResolutionMemo);
        }
    }
    Ok(())
}

#[contractimpl]
impl TreasuryContract {
    /// Raises a dispute against `settlement_id`, placing it on hold while the dispute is open.
    /// `expires_at` is a ledger UNIX timestamp (seconds) after which `expire_dispute` may be called.
    /// `evidence_hash` is an optional 32-byte hash of an off-chain evidence bundle
    /// (screenshots, messages, delivery proof) (#574): a verifiable pointer every signer
    /// can check against the same material, and a tamper-evident record for later audits.
    /// It is purely informational — never itself verified on-chain — and is returned
    /// unchanged by `get_dispute`. Pass `None` when there is no evidence to attach.
    /// Preconditions: contract not paused; `amount` must be positive.
    /// Errors: `ContractPaused`, `InvalidAmount`, `ArithmeticOverflow`.
    /// Emits: `dispute_raised`.
    pub fn raise_dispute(
        env: Env,
        claimant: Address,
        settlement_id: u64,
        counterparty: Address,
        amount: i128,
        expires_at: u64,
        evidence_hash: Option<BytesN<32>>,
    ) -> Result<u64, TreasuryError> {
        require_not_paused(&env);
        claimant.require_auth();
        if amount <= 0 {
            return Err(TreasuryError::InvalidAmount);
        }
        if let Some(mut settlement) = env
            .storage()
            .persistent()
            .get::<DataKey, Settlement>(&DataKey::Settlement(settlement_id))
        {
            if settlement.status == SettlementStatus::Pending {
                settlement.status = SettlementStatus::OnHold;
                write_settlement(&env, settlement_id, &settlement);
            }
        }
        let count: u64 = env
            .storage()
            .instance()
            .get(&DataKey::DisputeCount)
            .unwrap_or(0);
        let id = count
            .checked_add(1)
            .ok_or(TreasuryError::ArithmeticOverflow)?;
        let dispute = Dispute {
            id,
            settlement_id,
            claimant,
            counterparty,
            amount,
            status: DisputeStatus::Raised,
            resolution_approvals: Vec::new(&env),
            resolution_weight: 0,
            resolution_for_claimant: false,
            dispute_expires_at: expires_at,
            claimant_share_bps: 0,
            evidence_hash,
            resolution_memo: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(id), &dispute);
        env.storage().instance().set(&DataKey::DisputeCount, &id);
        let mut by_settlement: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::SettlementDisputes(settlement_id))
            .unwrap_or_else(|| Vec::new(&env));
        by_settlement.push_back(id);
        env.storage().persistent().set(
            &DataKey::SettlementDisputes(settlement_id),
            &by_settlement,
        );
        env.events()
            .publish((Symbol::new(&env, "dispute_raised"), id), dispute);
        Ok(id)
    }

    /// Transitions a `Raised` dispute to `Expired` after its deadline and releases the
    /// associated settlement from `OnHold` back to `Pending` once no other open disputes
    /// remain against it (mirrors the behaviour of `resolve_dispute`).
    /// Errors: `DisputeNotFound`, `DisputeAlreadyResolved`, `DisputeNotExpired`.
    /// Panics: `Unauthorized`.
    /// Emits: `dispute_expired`.
    pub fn expire_dispute(env: Env, admin: Address, dispute_id: u64) -> Result<(), TreasuryError> {
        require_admin(&env, &admin);
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(dispute_id))
            .ok_or(TreasuryError::DisputeNotFound)?;
        if dispute.status != DisputeStatus::Raised {
            return Err(TreasuryError::DisputeAlreadyResolved);
        }
        if env.ledger().timestamp() < dispute.dispute_expires_at {
            return Err(TreasuryError::DisputeNotExpired);
        }
        dispute.status = DisputeStatus::Expired;
        let settlement_id = dispute.settlement_id;
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(dispute_id), &dispute);
        env.events()
            .publish((Symbol::new(&env, "dispute_expired"), dispute_id), dispute);
        // Use the shared helper so the hold is only released once all open
        // disputes referencing this settlement have been resolved or expired.
        release_settlement_hold_if_no_open_disputes(&env, settlement_id);
        Ok(())
    }

    /// Returns the dispute with the given `dispute_id`.
    /// Errors: `DisputeNotFound`.
    pub fn get_dispute(env: Env, dispute_id: u64) -> Result<Dispute, TreasuryError> {
        env.storage()
            .persistent()
            .get(&DataKey::Dispute(dispute_id))
            .ok_or(TreasuryError::DisputeNotFound)
    }

    /// Returns every dispute raised against `settlement_id`, in the order they were
    /// raised. Bounded by the (small) number of disputes a single settlement can
    /// accumulate; returns an empty `Vec` when none have been raised.
    pub fn get_disputes_by_settlement(env: Env, settlement_id: u64) -> Vec<Dispute> {
        let ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::SettlementDisputes(settlement_id))
            .unwrap_or_else(|| Vec::new(&env));
        let mut disputes = Vec::new(&env);
        for id in ids.iter() {
            if let Some(dispute) = env
                .storage()
                .persistent()
                .get::<DataKey, Dispute>(&DataKey::Dispute(id))
            {
                disputes.push_back(dispute);
            }
        }
        disputes
    }

    /// Resolves an open dispute in favour of claimant or counterparty (admin-only).
    /// When the last open dispute for a settlement is resolved, the settlement hold is released.
    /// `memo` is an optional bounded rationale (max [`MAX_RESOLUTION_MEMO_LEN`] bytes) recorded
    /// on the dispute and included in the `dispute_resolved` event (#744).
    /// Errors: `DisputeNotFound`, `DisputeAlreadyResolved`, `ContractPaused`, `InvalidResolutionMemo`.
    /// Panics: `Unauthorized`.
    /// Emits: `dispute_resolved`.
    pub fn resolve_dispute(
        env: Env,
        admin: Address,
        dispute_id: u64,
        in_favor_of_claimant: bool,
        memo: Option<soroban_sdk::String>,
    ) -> Result<(), TreasuryError> {
        require_admin(&env, &admin);
        require_not_paused(&env);
        validate_resolution_memo(&memo)?;
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(dispute_id))
            .ok_or(TreasuryError::DisputeNotFound)?;
        if dispute.status != DisputeStatus::Raised {
            return Err(TreasuryError::DisputeAlreadyResolved);
        }
        dispute.status = if in_favor_of_claimant {
            DisputeStatus::ResolvedClaimant
        } else {
            DisputeStatus::ResolvedCounterparty
        };
        dispute.resolution_memo = memo;
        let settlement_id = dispute.settlement_id;
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(dispute_id), &dispute);
        env.events()
            .publish((Symbol::new(&env, "dispute_resolved"), dispute_id), dispute);
        release_settlement_hold_if_no_open_disputes(&env, settlement_id);
        Ok(())
    }

    /// Resolves an open dispute by splitting `dispute.amount` between claimant and
    /// counterparty according to `claimant_bps` (out of [`BPS_DENOMINATOR`]) instead of the
    /// binary claimant/counterparty outcome `resolve_dispute` supports (admin-only).
    ///
    /// Design note: like `resolve_dispute`, this only ever pays out `dispute.amount` (the
    /// amount raised with the dispute, not necessarily equal to the associated settlement's
    /// own `amount`) directly from the treasury's token balance to the two parties; it does
    /// not itself execute the settlement. Releasing the settlement hold below only means the
    /// settlement becomes eligible for its own `execute_settlement`/`partially_execute_settlement`
    /// payout again — callers must ensure the disputed amount and the settlement's payout are
    /// not double-counted when both eventually execute, exactly as with a binary resolution.
    /// `memo` is an optional bounded rationale (max [`MAX_RESOLUTION_MEMO_LEN`] bytes) recorded
    /// on the dispute and included in the `dispute_resolved_split` event (#744).
    /// Panics: `DisputeNotFound`, `DisputeAlreadyResolved`, `ContractPaused`, `InvalidSplitRatio`,
    /// `InvalidResolutionMemo`.
    /// Emits: `dispute_resolved_split`.
    pub fn resolve_dispute_split(
        env: Env,
        admin: Address,
        dispute_id: u64,
        claimant_bps: u32,
        token_contract: Address,
        memo: Option<soroban_sdk::String>,
    ) {
        require_admin(&env, &admin);
        require_not_paused(&env);
        if claimant_bps > BPS_DENOMINATOR {
            soroban_sdk::panic_with_error!(env, TreasuryError::InvalidSplitRatio);
        }
        if let Err(e) = validate_resolution_memo(&memo) {
            soroban_sdk::panic_with_error!(env, e);
        }
        let mut dispute: Dispute = env
            .storage()
            .persistent()
            .get(&DataKey::Dispute(dispute_id))
            .ok_or(TreasuryError::DisputeNotFound)
            .unwrap_or_else(|e| soroban_sdk::panic_with_error!(env, e));
        if dispute.status != DisputeStatus::Raised {
            soroban_sdk::panic_with_error!(env, TreasuryError::DisputeAlreadyResolved);
        }
        dispute.status = DisputeStatus::ResolvedSplit;
        dispute.claimant_share_bps = claimant_bps;
        dispute.resolution_memo = memo;
        let settlement_id = dispute.settlement_id;
        env.storage()
            .persistent()
            .set(&DataKey::Dispute(dispute_id), &dispute);
        env.events()
            .publish((Symbol::new(&env, "dispute_resolved_split"), dispute_id), dispute);
        release_settlement_hold_if_no_open_disputes(&env, settlement_id);
    }
}
