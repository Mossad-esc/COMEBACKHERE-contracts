// Event schema for Redis webhook delivery compatibility:
//
// Event Format:
// - Topics: Variable-length tuple of Symbols and data fields
// - Data: Serializable structs (Invoice, Address) or primitive types
//
// Redis Webhook Consumer Compatibility:
// All emitted events are compatible with JSON serialization for webhook delivery:
// - Symbol types serialize to strings
// - Address types serialize to account identifiers
// - Numeric types (u64, i128) serialize as JSON numbers or strings
// - Enum variants (InvoiceStatus) serialize to string representations
// - Structs (Invoice) serialize to JSON objects with field keys
// - Optional types (Option<u64>) serialize to null or value

use crate::invoice::Invoice;
use crate::template::InvoiceTemplate;
use soroban_sdk::{Address, Env, Symbol};

pub fn invoice_created(env: &Env, id: u64, invoice: &Invoice) {
    env.events()
        .publish((Symbol::new(env, "invoice_created"), id), invoice.clone());
}

pub fn invoice_paid(env: &Env, id: u64, invoice: &Invoice) {
    env.events()
        .publish((Symbol::new(env, "invoice_paid"), id), invoice.clone());
}

pub fn invoice_expired(env: &Env, id: u64, invoice: &Invoice) {
    env.events()
        .publish((Symbol::new(env, "invoice_expired"), id), invoice.clone());
}

pub fn invoice_cancelled(env: &Env, id: u64, invoice: &Invoice) {
    env.events()
        .publish((Symbol::new(env, "invoice_cancelled"), id), invoice.clone());
}

pub fn invoice_refund_requested(env: &Env, id: u64, invoice: &Invoice) {
    env.events().publish(
        (Symbol::new(env, "invoice_refund_req"), id),
        invoice.clone(),
    );
}

pub fn escrow_released(env: &Env, id: u64, invoice: &Invoice) {
    env.events()
        .publish((Symbol::new(env, "escrow_released"), id), invoice.clone());
}

pub fn template_created(env: &Env, id: u64, template: &InvoiceTemplate) {
    env.events()
        .publish((Symbol::new(env, "template_created"), id), template.clone());
}

/// A template was activated or deactivated: data is the new `active` flag.
pub fn template_activated(env: &Env, id: u64, active: bool) {
    env.events()
        .publish((Symbol::new(env, "template_activated"), id), active);
}

/// The keeper generated an invoice: topics are the template id and invoice id.
pub fn template_generated(env: &Env, template_id: u64, invoice_id: u64) {
    env.events().publish(
        (
            Symbol::new(env, "template_generated"),
            template_id,
            invoice_id,
        ),
        (),
    );
}

pub fn contract_paused(env: &Env, admin: &Address) {
    env.events()
        .publish((Symbol::new(env, "contract_paused"),), admin);
}

pub fn contract_unpaused(env: &Env, admin: &Address) {
    env.events()
        .publish((Symbol::new(env, "contract_unpaused"),), admin);
}
