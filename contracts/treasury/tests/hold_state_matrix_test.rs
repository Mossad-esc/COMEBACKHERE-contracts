use soroban_sdk::{testutils::Address as _, Address, Env, String};

use treasury::{TreasuryContract, TreasuryContractClient};

fn setup() -> (Env, TreasuryContractClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, TreasuryContract);
    let client = TreasuryContractClient::new(&env, &contract_id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client, admin)
}

#[test]
fn hold_then_force_cancel_releases_hold_state() {
    let (env, client, _admin) = setup();
    let settlement_id = String::from_str(&env, "settle-745");
    let reason = String::from_str(&env, "pending review");
    let expiry: u64 = 1_000;

    client.hold_settlement(&settlement_id, &reason, &expiry);

    assert_eq!(client.get_hold_reason(&settlement_id), Some(reason.clone()));
    assert_eq!(client.get_hold_expiry(&settlement_id), Some(expiry));

    client.force_cancel_settlement(&settlement_id);

    assert_eq!(client.get_hold_reason(&settlement_id), None);
    assert_eq!(client.get_hold_expiry(&settlement_id), None);
}
