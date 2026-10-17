//! Refund entrypoints for `InvoiceContract`, split out of `lifecycle.rs` for
//! readability (#719). No ABI change: soroban-sdk merges every submodule's
//! `#[contractimpl]` block into a single generated client.
//!
//! Note: the originating issue also named `process_refund`,
//! `calculate_net_refund` and `get_refund_breakdown` for this move, but none
//! of those entrypoints exist in this crate — only `request_refund`,
//! `approve_refund` and `reject_refund` do. Those three are moved here.

use crate::events;
use crate::validation::{require_admin, require_not_paused};
use crate::{append_history, DataKey, Invoice, InvoiceContract, InvoiceError, InvoiceStatus};
#[allow(unused_imports)]
use crate::{InvoiceContractArgs, InvoiceContractClient};
use crate::MaybeAddress;
use soroban_sdk::{contractimpl, Address, Env};

#[contractimpl]
impl InvoiceContract {
    // payer may request a refund on a paid invoice (escrow dispute)
    pub fn request_refund(env: Env, payer: Address, id: u64) -> Result<(), InvoiceError> {
        payer.require_auth();
        require_not_paused(&env)?;

        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;

        if invoice.status != InvoiceStatus::Paid {
            return Err(InvoiceError::NotPaid);
        }
        if invoice.payer != MaybeAddress::Some(payer.clone()) {
            return Err(InvoiceError::Unauthorized);
        }

        invoice.status = InvoiceStatus::RefundRequested;
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        append_history(
            &env,
            id,
            InvoiceStatus::Paid,
            InvoiceStatus::RefundRequested,
        );
        events::invoice_refund_requested(&env, id, &invoice);
        Ok(())
    }

    /// Approve a refund request. Admin-only. Transitions RefundRequested → Refunded.
    pub fn approve_refund(env: Env, admin: Address, id: u64) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        require_not_paused(&env)?;

        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;

        if invoice.status != InvoiceStatus::RefundRequested {
            return Err(InvoiceError::NotRefundRequested);
        }

        invoice.status = InvoiceStatus::Refunded;
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        append_history(
            &env,
            id,
            InvoiceStatus::RefundRequested,
            InvoiceStatus::Refunded,
        );
        events::refund_approved(&env, id, &invoice);
        Ok(())
    }

    /// Reject a refund request. Admin-only. Transitions RefundRequested → Paid.
    pub fn reject_refund(env: Env, admin: Address, id: u64) -> Result<(), InvoiceError> {
        require_admin(&env, &admin)?;
        require_not_paused(&env)?;

        let mut invoice: Invoice = env
            .storage()
            .persistent()
            .get(&DataKey::Invoice(id))
            .ok_or(InvoiceError::NotFound)?;
        if invoice.status != InvoiceStatus::RefundRequested {
            return Err(InvoiceError::NotRefundRequested);
        }

        invoice.status = InvoiceStatus::Paid;
        env.storage()
            .persistent()
            .set(&DataKey::Invoice(id), &invoice);
        append_history(
            &env,
            id,
            InvoiceStatus::RefundRequested,
            InvoiceStatus::Paid,
        );
        events::refund_rejected(&env, id, &invoice);
        Ok(())
    }
}
