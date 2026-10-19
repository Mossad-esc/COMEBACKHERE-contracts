use crate::{require_admin, require_not_paused, DataKey, TreasuryContract, TreasuryError};
#[allow(unused_imports)]
use crate::{TreasuryContractArgs, TreasuryContractClient};
use multisig::signer_weight;
use soroban_sdk::{contractimpl, token, Address, Bytes, Env, Symbol, Vec};

#[contractimpl]
impl TreasuryContract {
    /// Deposits `amount` tokens from `from` into the treasury via `token_contract`.
    /// An optional `reference` string can be supplied for off-chain reconciliation;
    /// it is included in the `deposit` event so finance systems can match deposits
    /// to invoices or external transfers automatically.
    /// Errors: `ContractPaused`, `InvalidAmount`.
    /// Emits: `deposit`.
    pub fn deposit(
        env: Env,
        from: Address,
        token_contract: Address,
        amount: i128,
        reference: Option<Bytes>,
    ) -> Result<(), TreasuryError> {
        require_not_paused(&env);
        from.require_auth();
        deposit_one(&env, &from, &token_contract, amount, reference)
    }

    /// Deposits multiple `(token_contract, amount)` pairs from `from` into the treasury.
    /// An optional `reference` string is forwarded to every `deposit` event emitted
    /// by the batch, allowing the whole batch to be tagged with a single reconciliation id.
    /// Errors: `ContractPaused`, `InvalidAmount`.
    /// Emits: `deposit` for each deposited token.
    pub fn batch_deposit(
        env: Env,
        from: Address,
        deposits: Vec<(Address, i128)>,
        reference: Option<Bytes>,
    ) -> Result<(), TreasuryError> {
        require_not_paused(&env);
        from.require_auth();
        for (token_contract, amount) in deposits.iter() {
            deposit_one(&env, &from, &token_contract, amount, reference.clone())?;
        }
        Ok(())
    }

    /// Withdraws `amount` tokens from the treasury to `to` via `token_contract`.
    /// Errors: `ContractPaused`, `InvalidAmount`, `InsufficientBalance`, `DestinationNotAllowed`.
    /// Emits: `withdraw`.
    pub fn withdraw(
        env: Env,
        to: Address,
        token_contract: Address,
        amount: i128,
    ) -> Result<(), TreasuryError> {
        require_not_paused(&env);
        to.require_auth();
        if amount <= 0 {
            return Err(TreasuryError::InvalidAmount);
        }
        // Check withdrawal destination allowlist: if non-empty, `to` must be present.
        let allowlist: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::WithdrawalAllowlist)
            .unwrap_or_else(|| Vec::new(&env));
        if !allowlist.is_empty() && !allowlist.contains(&to) {
            return Err(TreasuryError::DestinationNotAllowed);
        }
        let mut balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::Balance(to.clone(), token_contract.clone()))
            .unwrap_or(0);
        if balance < amount {
            return Err(TreasuryError::InsufficientBalance);
        }
        enforce_withdrawal_limit(&env, &to, amount);
        balance = balance
            .checked_sub(amount)
            .ok_or(TreasuryError::ArithmeticOverflow)?;
        env.storage().persistent().set(
            &DataKey::Balance(to.clone(), token_contract.clone()),
            &balance,
        );
        // Decrement the aggregate per-token total (#742).
        let total: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TokenTotal(token_contract.clone()))
            .unwrap_or(0);
        let new_total = total
            .checked_sub(amount)
            .ok_or(TreasuryError::ArithmeticOverflow)?;
        env.storage()
            .persistent()
            .set(&DataKey::TokenTotal(token_contract.clone()), &new_total);
        let treasury = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_contract);
        token_client.transfer(&treasury, &to, &amount);
        env.events()
            .publish((Symbol::new(&env, "withdraw"), to), amount);
        Ok(())
    }

    /// Returns the recorded deposit balance for `address` under `token_contract`, or 0 if never
    /// deposited. Balances are segregated per token contract (#448); this never mixes holdings
    /// across different allowlisted tokens.
    /// Read-only, no authentication required.
    pub fn get_balance(env: Env, address: Address, token_contract: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(address, token_contract))
            .unwrap_or(0)
    }

    /// Returns `address`'s recorded deposit balance for every currently-allowed
    /// token in a single call (#566), as `(token_contract, balance)` pairs.
    ///
    /// Solves the dashboard/health-script problem of making one `get_balance`
    /// call per token — which is slow and can observe an inconsistent snapshot
    /// if a deposit lands between calls. The output is bounded by the allowed
    /// token list (capped at `MAX_ALLOWED_TOKENS`), so this can never exceed its
    /// budget regardless of how many tokens are allowed.
    /// Read-only, no authentication required.
    pub fn get_all_balances(env: Env, address: Address) -> Vec<(Address, i128)> {
        let tokens = TreasuryContract::get_allowed_tokens(env.clone());
        let mut result = Vec::new(&env);
        for token_contract in tokens.iter() {
            let balance: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::Balance(address.clone(), token_contract.clone()))
                .unwrap_or(0);
            result.push_back((token_contract, balance));
        }
        result
    }

    /// Drains the full token balance of the treasury to `recipient` (admin-only, paused-only emergency drain).
    /// Errors: `NotPaused`.
    /// Panics: `Unauthorized`.
    /// Emits: `treasury_drained`.
    pub fn withdraw_all(
        env: Env,
        admin: Address,
        token_contract: Address,
        recipient: Address,
    ) -> Result<(), TreasuryError> {
        require_admin(&env, &admin);
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if !paused {
            return Err(TreasuryError::NotPaused);
        }
        let treasury = env.current_contract_address();
        let token_client = token::Client::new(&env, &token_contract);
        let balance = token_client.balance(&treasury);
        if balance > 0 {
            enforce_withdrawal_limit(&env, &recipient, balance);
            token_client.transfer(&treasury, &recipient, &balance);
        }
        env.events()
            .publish((Symbol::new(&env, "treasury_drained"),), recipient);
        Ok(())
    }

    /// Moves **all** funds held in `token_contract` to `recovery_address` as an
    /// emergency escape hatch when the treasury is compromised or a critical bug
    /// is found.
    ///
    /// This function requires **every registered signer** (full quorum, not just
    /// the normal approval threshold) to have called `require_auth`, and it only
    /// executes while the contract is paused. The combination of "paused" and
    /// "full quorum" means a single attacker controlling fewer than all signers
    /// cannot drain the treasury through this path.
    ///
    /// Steps for safe use:
    /// 1. Pause the contract with `pause`.
    /// 2. Collect `require_auth` signatures from **all** registered signers.
    /// 3. Call `emergency_withdraw` with those authorisations and a pre-agreed
    ///    `recovery_address`.
    ///
    /// Preconditions:
    /// * Contract must be paused (`Errors: NotPaused`).
    /// * Every address in `SignerList` with weight > 0 must authenticate.
    ///   (`Panics: UnauthorizedSigner` for the first missing authorisation.)
    ///
    /// Emits: **`emergency_withdraw`** — HIGH-SEVERITY event (see
    /// `docs/alerting-guide.md`); carries `(recovery_address, amount)`.
    pub fn emergency_withdraw(
        env: Env,
        signers: Vec<Address>,
        token_contract: Address,
        recovery_address: Address,
    ) -> Result<(), TreasuryError> {
        // 1. Only allowed while paused.
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::Paused)
            .unwrap_or(false);
        if !paused {
            return Err(Treasu

/* … truncated 4941 chars — edit only what you need near the top … */
