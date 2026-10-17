#![no_std]

use compliance_client::ComplianceClient;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};
use treasury::TreasuryContractClient;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
}

/// Reference on-chain implementation of the `SettlementWorkflow` role described in
/// `ARCHITECTURE.md`: gates `Treasury::execute_settlement` behind
/// `Compliance::is_allowed`. Treasury does not consult compliance itself, so this
/// contract is the enforcement point for the compliance gate in the payment lifecycle.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum WorkflowError {
    ComplianceFailed = 1,
    AlreadyInitialized = 2,
    NotInitialized = 3,
}

#[contract]
pub struct SettlementWorkflowContract;

#[contractimpl]
impl SettlementWorkflowContract {
    /// Records `admin` as this contract's administrator. Optional: callers that only use
    /// `execute_with_compliance` do not need to call this. Errors if already initialized.
    pub fn initialize(env: Env, admin: Address) -> Result<(), WorkflowError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(WorkflowError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    /// Returns the stored admin address.
    /// Errors: `NotInitialized` if `initialize` has not been called.
    pub fn get_admin(env: Env) -> Result<Address, WorkflowError> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(WorkflowError::NotInitialized)
    }

    /// Checks `Compliance::is_allowed(merchant)` and, only if it passes, calls
    /// `Treasury::execute_settlement(..., settlement_id, token_contract)` using this
    /// contract's own address as the authorizing signer (it must be registered as a
    /// Treasury signer via `Treasury::set_signer` beforehand).
    /// Returns `Err(WorkflowError::ComplianceFailed)` without touching Treasury
    /// if the compliance check fails.
    pub fn execute_with_compliance(
        env: Env,
        compliance_id: Address,
        treasury_id: Address,
        settlement_id: u64,
        token_contract: Address,
        merchant: Address,
    ) -> Result<(), WorkflowError> {
        let compliance = ComplianceClient::new(&env, &compliance_id);
        if !compliance.is_allowed(&merchant) {
            return Err(WorkflowError::ComplianceFailed);
        }
        let treasury = TreasuryContractClient::new(&env, &treasury_id);
        treasury.execute_settlement(
            &env.current_contract_address(),
            &settlement_id,
            &token_contract,
        );
        Ok(())
    }
}
