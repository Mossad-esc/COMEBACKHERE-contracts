//! Recurring invoice template entrypoints: template writes and reads, plus the
//! permissionless keeper that turns due templates into invoices.

use super::lifecycle::create_invoice_record;
use crate::events;
use crate::invoice::{DataKey, InvoiceError, MaybeBytes};
use crate::template::{InvoiceTemplate, RecurrenceInterval, MAX_BATCH_SIZE, MAX_PAGE_LIMIT};
use crate::validation::{require_not_paused, require_positive_amount, require_usdc_precision};
use soroban_sdk::{vec, Address, Env, Vec};

/// Registers a recurring invoice template for `merchant` and returns its id.
/// Errors: `ContractPaused`, `InvalidAmount`, `AmountPrecision`, `ZeroDuration`,
///         `ExpiryOverflow`, `InvoiceIdOverflow`.
/// Emits: `template_created`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_template(
    env: &Env,
    merchant: Address,
    amount_usdc: i128,
    gross_usdc: i128,
    interval: RecurrenceInterval,
    expires_in_seconds: u64,
    metadata_hash: MaybeBytes,
    payment_link_hash: MaybeBytes,
) -> Result<u64, InvoiceError> {
    merchant.require_auth();
    require_not_paused(env)?;
    require_positive_amount(amount_usdc, gross_usdc)?;
    require_usdc_precision(amount_usdc, gross_usdc)?;
    if expires_in_seconds == 0 {
        return Err(InvoiceError::ZeroDuration);
    }

    let count: u64 = env
        .storage()
        .instance()
        .get(&DataKey::TemplateCount)
        .unwrap_or(0);
    let id = count
        .checked_add(1)
        .ok_or(InvoiceError::InvoiceIdOverflow)?;
    // First run is one full period away, so a template is never due on creation.
    let next_run_at = env
        .ledger()
        .timestamp()
        .checked_add(interval.period_seconds())
        .ok_or(InvoiceError::ExpiryOverflow)?;

    let template = InvoiceTemplate {
        id,
        merchant: merchant.clone(),
        amount_usdc,
        gross_usdc,
        interval,
        expires_in_seconds,
        metadata_hash,
        payment_link_hash,
        active: true,
        next_run_at,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Template(id), &template);
    env.storage().instance().set(&DataKey::TemplateCount, &id);
    index_template(env, &merchant, id);
    events::template_created(env, id, &template);
    Ok(id)
}

/// Returns the template registered under `id`.
/// Errors: `NotFound`.
pub(crate) fn get_invoice_template(env: &Env, id: u64) -> Result<InvoiceTemplate, InvoiceError> {
    env.storage()
        .persistent()
        .get(&DataKey::Template(id))
        .ok_or(InvoiceError::NotFound)
}

/// Returns one page of `merchant`'s templates in creation order, skipping the
/// first `start` entries. `limit` is capped at [`MAX_PAGE_LIMIT`].
pub(crate) fn list_templates_by_merchant(
    env: &Env,
    merchant: Address,
    start: u64,
    limit: u64,
) -> Vec<InvoiceTemplate> {
    let capped = core::cmp::min(limit, MAX_PAGE_LIMIT as u64);
    let ids: Vec<u64> = env
        .storage()
        .instance()
        .get(&DataKey::MerchantTemplates(merchant))
        .unwrap_or_else(|| vec![env]);

    let mut page = Vec::new(env);
    let mut skipped: u64 = 0;
    let len = ids.len();
    for i in 0..len {
        if (page.len() as u64) >= capped {
            break;
        }
        let Some(id) = ids.get(i) else { continue };
        if skipped < start {
            skipped += 1;
            continue;
        }
        if let Some(template) = env
            .storage()
            .persistent()
            .get::<DataKey, InvoiceTemplate>(&DataKey::Template(id))
        {
            page.push_back(template);
        }
    }
    page
}

/// Activates or deactivates a template (merchant or admin). Deactivated
/// templates keep their schedule but are skipped by the keeper.
/// Errors: `NotFound`, `Unauthorized`.
/// Emits: `template_activated`.
pub(crate) fn set_template_active(
    env: &Env,
    caller: &Address,
    id: u64,
    active: bool,
) -> Result<(), InvoiceError> {
    let mut template: InvoiceTemplate = env
        .storage()
        .persistent()
        .get(&DataKey::Template(id))
        .ok_or(InvoiceError::NotFound)?;
    let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
    if *caller != template.merchant && *caller != admin {
        return Err(InvoiceError::Unauthorized);
    }
    template.active = active;
    env.storage()
        .persistent()
        .set(&DataKey::Template(id), &template);
    events::template_activated(env, id, active);
    Ok(())
}

/// Permissionless keeper entrypoint: creates one invoice for every template in
/// `template_ids` that is due at the current ledger timestamp, advancing each
/// schedule past `now`. Unknown and not-yet-due templates are skipped, so a
/// second call inside the same period generates nothing.
/// Errors: `BatchTooLarge` when more than [`MAX_BATCH_SIZE`] ids are supplied.
/// Emits: `template_generated` once per generated invoice.
pub(crate) fn generate_due_invoices(
    env: &Env,
    template_ids: Vec<u64>,
) -> Result<u32, InvoiceError> {
    if template_ids.len() > MAX_BATCH_SIZE {
        return Err(InvoiceError::BatchTooLarge);
    }
    require_not_paused(env)?;

    let now = env.ledger().timestamp();
    let mut generated: u32 = 0;
    for i in 0..template_ids.len() {
        let Some(id) = template_ids.get(i) else {
            continue;
        };
        let Some(mut template) = env
            .storage()
            .persistent()
            .get::<DataKey, InvoiceTemplate>(&DataKey::Template(id))
        else {
            continue;
        };
        if !template.active || now < template.next_run_at {
            continue;
        }

        let invoice_id = create_invoice_record(
            env,
            &template.merchant,
            template.amount_usdc,
            template.gross_usdc,
            template.expires_in_seconds,
            template.metadata_hash.clone(),
            template.payment_link_hash.clone(),
            0,
        )?;

        // Advance past every period already elapsed so the next run is in the
        // future: one call can never mint two invoices for the same period.
        let period = template.interval.period_seconds();
        let mut next_run_at = template.next_run_at;
        while next_run_at <= now {
            match next_run_at.checked_add(period) {
                Some(advanced) => next_run_at = advanced,
                None => {
                    next_run_at = u64::MAX;
                    break;
                }
            }
        }
        template.next_run_at = next_run_at;
        env.storage()
            .persistent()
            .set(&DataKey::Template(id), &template);
        events::template_generated(env, id, invoice_id);
        generated += 1;
    }
    Ok(generated)
}

/// Appends `id` to the merchant's template index, preserving creation order.
fn index_template(env: &Env, merchant: &Address, id: u64) {
    let key = DataKey::MerchantTemplates(merchant.clone());
    let mut ids: Vec<u64> = env
        .storage()
        .instance()
        .get(&key)
        .unwrap_or_else(|| vec![env]);
    ids.push_back(id);
    env.storage().instance().set(&key, &ids);
}
