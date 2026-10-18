use soroban_sdk::{testutils::Ledger, Address, BytesN, Env, String};

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

fn ref_id(env: &Env, seed: u8) -> String {
    String::from_str(env, "ref-0001")
}

#[test]
fn deposit_rejects_duplicate_reference_id() {
    let (env, client, admin) = setup();
    let ref_id = ref_id(&env, 1);

    client.deposit(&admin, &100, &ref_id);

    let result = client.try_deposit(&admin, &100, &ref_id);
    assert!(result.is_err(), "duplicate reference id must be rejected");
}

#[test]
fn deposit_allows_reference_id_after_ttl() {
    let (env, client, admin) = setup();
    let ref_id = ref_id(&env, 1);

    client.deposit(&admin, &100, &ref_id);

    // Advance the ledger past the reference-id TTL window.
    env.ledger().with_mut(|li| {
        li.sequence_number += 100_000;
        li.timestamp += 100_000;
    });

    let result = client.try_deposit(&admin, &100, &ref_id);
    assert!(result.is_ok(), "reference id should be reusable after TTL");
}

#[test]
fn deposit_allows_distinct_reference_ids() {
    let (env, client, admin) = setup();

    client.deposit(&admin, &100, &String::from_str(&env, "ref-a"));
    client.deposit(&admin, &100, &String::from_str(&env, "ref-b"));
}
