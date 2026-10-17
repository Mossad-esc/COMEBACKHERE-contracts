// Issue #692: signer activity is stored under `DataKey::SignerLastActive` and
// exposed through `get_signer_last_active`.
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};
use treasury::{TreasuryContract, TreasuryContractClient};

fn setup(env: &Env) -> (TreasuryContractClient<'static>, Address, Address, Address) {
    env.mock_all_auths();
    let admin = Address::generate(env);
    let signer = Address::generate(env);
    let merchant = Address::generate(env);
    let id = env.register_contract(None, TreasuryContract);
    let client = TreasuryContractClient::new(env, &id);
    client.initialize(&admin, &2);
    client.set_signer(&admin, &signer, &1);
    (client, admin, signer, merchant)
}

#[test]
fn signer_without_approvals_has_no_last_active_timestamp() {
    let env = Env::default();
    let (client, _admin, signer, _merchant) = setup(&env);

    assert_eq!(client.get_signer_last_active(&signer), 0);
}

#[test]
fn approving_a_settlement_records_last_active() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let (client, admin, signer, merchant) = setup(&env);

    assert_eq!(client.get_signer_last_active(&signer), 0);

    let settlement_id = client.propose_settlement(&admin, &merchant, &10_000_000);
    client.approve_settlement(&signer, &settlement_id);

    assert_eq!(client.get_signer_last_active(&signer), 1_000);
}

#[test]
fn partial_approval_records_last_active() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 4_242);
    let (client, admin, signer, merchant) = setup(&env);

    let settlement_id = client.propose_settlement(&admin, &merchant, &10_000_000);
    client.approve_partial_settlement(&signer, &settlement_id, &4_000_000);

    assert_eq!(client.get_signer_last_active(&signer), 4_242);
}

#[test]
fn dispute_vote_records_last_active() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 7_777);
    let (client, admin, signer, merchant) = setup(&env);

    let settlement_id = client.propose_settlement(&admin, &merchant, &10_000_000);
    let counterparty = Address::generate(&env);
    let dispute_id = client.raise_dispute(&admin, &settlement_id, &counterparty, &10_000_000);

    assert_eq!(client.get_signer_last_active(&signer), 0);

    client.vote_dispute_resolution(&signer, &dispute_id, &true);

    assert_eq!(client.get_signer_last_active(&signer), 7_777);
}

#[test]
fn signer_rotation_approval_records_last_active() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 9_001);
    let (client, admin, signer, _merchant) = setup(&env);
    let new_signer = Address::generate(&env);

    let rotation_id = client.propose_signer_rotation(&admin, &signer, &new_signer);
    assert_eq!(client.get_signer_last_active(&signer), 0);

    client.approve_signer_rotation(&signer, &rotation_id);

    assert_eq!(client.get_signer_last_active(&signer), 9_001);
}

#[test]
fn later_approval_refreshes_last_active() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let (client, admin, signer, merchant) = setup(&env);

    let first = client.propose_settlement(&admin, &merchant, &10_000_000);
    client.approve_settlement(&signer, &first);
    assert_eq!(client.get_signer_last_active(&signer), 1_000);

    env.ledger().with_mut(|l| l.timestamp = 5_000);
    let second = client.propose_settlement(&admin, &merchant, &10_000_000);
    client.approve_settlement(&signer, &second);

    assert_eq!(client.get_signer_last_active(&signer), 5_000);
}

#[test]
fn repeated_approval_of_the_same_item_does_not_refresh_last_active() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let (client, admin, signer, merchant) = setup(&env);

    let settlement_id = client.propose_settlement(&admin, &merchant, &10_000_000);
    client.approve_settlement(&signer, &settlement_id);

    env.ledger().with_mut(|l| l.timestamp = 2_500);
    // Duplicate approval: the weight is not re-counted, so no new activity is stamped.
    client.approve_settlement(&signer, &settlement_id);

    assert_eq!(client.get_signer_last_active(&signer), 1_000);
}

#[test]
fn activity_is_tracked_per_signer() {
    let env = Env::default();
    env.ledger().with_mut(|l| l.timestamp = 300);
    let (client, admin, signer, merchant) = setup(&env);
    let other_signer = Address::generate(&env);
    client.set_signer(&admin, &other_signer, &1);

    let settlement_id = client.propose_settlement(&admin, &merchant, &10_000_000);
    client.approve_settlement(&signer, &settlement_id);

    assert_eq!(client.get_signer_last_active(&signer), 300);
    assert_eq!(client.get_signer_last_active(&other_signer), 0);

    env.ledger().with_mut(|l| l.timestamp = 800);
    client.approve_settlement(&other_signer, &settlement_id);

    assert_eq!(client.get_signer_last_active(&signer), 300);
    assert_eq!(client.get_signer_last_active(&other_signer), 800);
}
