//! Boundary tests for the bulk-operation cooldown (`BULK_OP_COOLDOWN_SECS`).
//!
//! Bulk allow/block operations share a cooldown window. These tests exercise the
//! exact boundary: `BULK_OP_COOLDOWN_SECS - 1`, `BULK_OP_COOLDOWN_SECS`, and
//! `BULK_OP_COOLDOWN_SECS + 1` seconds after a bulk operation, and assert whether
//! the cooldown is enforced per caller or globally.

use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

use compliance::{ComplianceContract, ComplianceContractClient, Error};

const BULK_OP_COOLDOWN_SECS: u64 = 60;

fn setup<'a>(env: &'a Env) -> (ComplianceContractClient<'a>, Address, Address) {
    let admin = Address::generate(env);
    let contract_id = env.register_contract(None, ComplianceContract);
    let client = ComplianceContractClient::new(env, &contract_id);
    client.initialize(&admin);
    let caller = Address::generate(env);
    (client, admin, caller)
}

fn addresses(env: &Env, count: u32) -> Vec<Address> {
    let mut out = Vec::new(env);
    for _ in 0..count {
        out.push_back(Address::generate(env));
    }
    out
}

#[test]
fn bulk_allow_cooldown_boundary() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, caller) = setup(&env);

    let first = addresses(&env, 2);
    client.bulk_allow(&caller, &first);

    // cooldown - 1: still within the cooldown window, must be rejected.
    env.ledger().set_timestamp(env.ledger().timestamp() + BULK_OP_COOLDOWN_SECS - 1);
    let second = addresses(&env, 2);
    let err = client.try_bulk_allow(&caller, &second).unwrap_err().unwrap();
    assert_eq!(err, Error::BulkOpCooldownActive);

    // exactly cooldown: window has elapsed, must be accepted.
    env.ledger().set_timestamp(env.ledger().timestamp() + 1);
    let third = addresses(&env, 2);
    client.bulk_allow(&caller, &third);

    // cooldown + 1: well past the window, must be accepted.
    env.ledger().set_timestamp(env.ledger().timestamp() + BULK_OP_COOLDOWN_SECS + 1);
    let fourth = addresses(&env, 2);
    client.bulk_allow(&caller, &fourth);
}

#[test]
fn bulk_block_cooldown_boundary() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, caller) = setup(&env);

    let first = addresses(&env, 2);
    client.bulk_block(&caller, &first);

    // cooldown - 1: still within the cooldown window, must be rejected.
    env.ledger().set_timestamp(env.ledger().timestamp() + BULK_OP_COOLDOWN_SECS - 1);
    let second = addresses(&env, 2);
    let err = client.try_bulk_block(&caller, &second).unwrap_err().unwrap();
    assert_eq!(err, Error::BulkOpCooldownActive);

    // exactly cooldown: window has elapsed, must be accepted.
    env.ledger().set_timestamp(env.ledger().timestamp() + 1);
    let third = addresses(&env, 2);
    client.bulk_block(&caller, &third);

    // cooldown + 1: well past the window, must be accepted.
    env.ledger().set_timestamp(env.ledger().timestamp() + BULK_OP_COOLDOWN_SECS + 1);
    let fourth = addresses(&env, 2);
    client.bulk_block(&caller, &fourth);
}

#[test]
fn bulk_cooldown_is_per_caller() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, caller_a) = setup(&env);
    let caller_b = Address::generate(&env);

    let first = addresses(&env, 2);
    client.bulk_allow(&caller_a, &first);

    // A different caller is not affected by caller_a's cooldown.
    let second = addresses(&env, 2);
    client.bulk_allow(&caller_b, &second);

    // caller_a is still within its own cooldown window.
    env.ledger().set_timestamp(env.ledger().timestamp() + BULK_OP_COOLDOWN_SECS - 1);
    let third = addresses(&env, 2);
    let err = client.try_bulk_allow(&caller_a, &third).unwrap_err().unwrap();
    assert_eq!(err, Error::BulkOpCooldownActive);

    // caller_b, having operated at the same time, is also within its own window.
    let fourth = addresses(&env, 2);
    let err = client.try_bulk_allow(&caller_b, &fourth).unwrap_err().unwrap();
    assert_eq!(err, Error::BulkOpCooldownActive);
}
