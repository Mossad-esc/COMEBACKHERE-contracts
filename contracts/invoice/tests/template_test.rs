//! Tests for #694: invoice template entrypoints.
//!
//! Verifies:
//! * Merchants can create templates.
//! * Invoices can be generated from templates.
//! * Templates can be disabled; disabled templates reject generation.
//! * Non-owning merchants are rejected for generate/disable.
//! * get_template returns correct data; NotFound for unknown IDs.
//! * Pagination of merchant templates works.
//! * Paused contract rejects all template operations.

use invoice::{
    InvoiceContract, InvoiceContractClient, InvoiceError, InvoiceStatus, MaybeAddress, MaybeBytes,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

extern crate std;

fn setup() -> (Env, Address, InvoiceContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register_contract(None, InvoiceContract);
    let client = InvoiceContractClient::new(&env, &id);
    client.initialize(&admin);
    (env, admin, client)
}

// ─── create_template ──────────────────────────────────────────────────────────

#[test]
fn test_create_template_succeeds_and_returns_id() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    assert_eq!(template_id, 1);
    assert_eq!(client.get_template_count(), 1);
}

#[test]
fn test_create_template_stores_correct_fields() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &20_000_000,
        &21_000_000,
        &7200,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    let template = client.get_template(&template_id);
    assert_eq!(template.id, template_id);
    assert_eq!(template.merchant, merchant);
    assert_eq!(template.amount_usdc, 20_000_000);
    assert_eq!(template.gross_usdc, 21_000_000);
    assert_eq!(template.duration_seconds, 7200);
    assert!(template.enabled);
}

#[test]
fn test_create_template_zero_duration_is_rejected() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let err = client
        .try_create_template(
            &merchant,
            &10_000_000,
            &10_250_000,
            &0,
            &MaybeBytes::None,
            &MaybeAddress::None,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, InvoiceError::ZeroDuration);
}

#[test]
fn test_create_template_invalid_amount_is_rejected() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    // amount_usdc > gross_usdc is invalid
    let err = client
        .try_create_template(
            &merchant,
            &20_000_000,
            &10_000_000,
            &3600,
            &MaybeBytes::None,
            &MaybeAddress::None,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, InvoiceError::InvalidAmount);
}

#[test]
fn test_create_template_emits_template_created_event() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{Symbol, TryFromVal};

    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );

    let events = env.events().all();
    let (_, topics, _) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get_unchecked(0)).unwrap(),
        Symbol::new(&env, "template_created")
    );
}

// ─── generate_from_template ───────────────────────────────────────────────────

#[test]
fn test_generate_from_template_creates_invoice() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    let invoice_id = client.generate_from_template(&merchant, &template_id, &0);
    let invoice = client.get_invoice(&invoice_id);
    assert_eq!(invoice.merchant, merchant);
    assert_eq!(invoice.amount_usdc, 10_000_000);
    assert_eq!(invoice.gross_usdc, 10_250_000);
    assert_eq!(invoice.status, InvoiceStatus::Pending);
}

#[test]
fn test_generate_from_template_emits_template_generated_event() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{Symbol, TryFromVal};

    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    client.generate_from_template(&merchant, &template_id, &0);

    let events = env.events().all();
    let (_, topics, _) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get_unchecked(0)).unwrap(),
        Symbol::new(&env, "template_generated")
    );
}

#[test]
fn test_generate_from_template_by_non_owner_is_rejected() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let other_merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    let err = client
        .try_generate_from_template(&other_merchant, &template_id, &0)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, InvoiceError::Unauthorized);
}

#[test]
fn test_generate_from_template_not_found_is_rejected() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let err = client
        .try_generate_from_template(&merchant, &999, &0)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, InvoiceError::NotFound);
}

// ─── disable_template ─────────────────────────────────────────────────────────

#[test]
fn test_disable_template_by_owner_succeeds() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    client.disable_template(&merchant, &template_id);
    let template = client.get_template(&template_id);
    assert!(!template.enabled);
}

#[test]
fn test_disabled_template_rejects_generation() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    client.disable_template(&merchant, &template_id);
    let err = client
        .try_generate_from_template(&merchant, &template_id, &0)
        .unwrap_err()
        .unwrap();
    // Disabled template is treated as not-pending
    assert_eq!(err, InvoiceError::NotPending);
}

#[test]
fn test_disable_template_by_non_owner_is_rejected() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let other = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    let err = client
        .try_disable_template(&other, &template_id)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, InvoiceError::Unauthorized);
}

#[test]
fn test_disable_template_emits_template_disabled_event() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{Symbol, TryFromVal};

    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    client.disable_template(&merchant, &template_id);

    let events = env.events().all();
    let (_, topics, _) = events.last().unwrap();
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get_unchecked(0)).unwrap(),
        Symbol::new(&env, "template_disabled")
    );
}

// ─── get_templates_by_merchant ────────────────────────────────────────────────

#[test]
fn test_get_templates_by_merchant_paginates() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);

    let t1 = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    let t2 = client.create_template(
        &merchant,
        &20_000_000,
        &21_000_000,
        &7200,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    let t3 = client.create_template(
        &merchant,
        &30_000_000,
        &31_000_000,
        &10800,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );

    let page1 = client.get_templates_by_merchant(&merchant, &0, &2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap(), t1);
    assert_eq!(page1.get(1).unwrap(), t2);

    let page2 = client.get_templates_by_merchant(&merchant, &2, &2);
    assert_eq!(page2.len(), 1);
    assert_eq!(page2.get(0).unwrap(), t3);
}

#[test]
fn test_get_templates_by_merchant_empty_for_new_merchant() {
    let (env, _admin, client) = setup();
    let merchant = Address::generate(&env);
    let page = client.get_templates_by_merchant(&merchant, &0, &10);
    assert_eq!(page.len(), 0);
}

// ─── paused contract ──────────────────────────────────────────────────────────

#[test]
fn test_create_template_when_paused_is_rejected() {
    let (env, admin, client) = setup();
    let merchant = Address::generate(&env);
    client.pause(&admin);
    let err = client
        .try_create_template(
            &merchant,
            &10_000_000,
            &10_250_000,
            &3600,
            &MaybeBytes::None,
            &MaybeAddress::None,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, InvoiceError::ContractPaused);
}

#[test]
fn test_generate_from_template_when_paused_is_rejected() {
    let (env, admin, client) = setup();
    let merchant = Address::generate(&env);
    let template_id = client.create_template(
        &merchant,
        &10_000_000,
        &10_250_000,
        &3600,
        &MaybeBytes::None,
        &MaybeAddress::None,
    );
    client.pause(&admin);
    let err = client
        .try_generate_from_template(&merchant, &template_id, &0)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, InvoiceError::ContractPaused);
}
