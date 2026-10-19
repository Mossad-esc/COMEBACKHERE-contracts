# Event Schema Reference

(#460) A single, formally-structured reference for every event emitted across all four
contracts in this workspace, for off-chain indexers to build a reliable parser against
without cross-referencing four READMEs or reading source. Every row below was derived
directly from the `env.events().publish(...)` call sites in source (not copied from
README prose), cross-checked against each contract's own README, with any discrepancy
found fixed as part of the same change (see `contracts/invoice/README.md`: `reject_refund`
and its `refund_rejected` event were implemented in source but missing from the README
entirely).

## How to read this document

Every Soroban event has two parts:

- **Topics** — a fixed-shape tuple. The first topic is always a `Symbol` naming the
  event; some events append a second topic (typically the entity ID) for cheap
  server-side filtering without decoding the data payload.
- **Data** — the event body, given as a Rust type. `Address`, `u32`, `u64`, `i128`,
  `bool`, and `Option<T>` are native Soroban SDK/XDR types. A named struct/enum links to
  its field list below the table it appears in the first time.

Topic tuples and data payloads are listed exactly as constructed in source
(`Symbol::new(&env, "...")`, the field/argument passed to `.publish(...)`), not
paraphrased.

This document should be kept in sync with source the same way `abis/*.json` is: any PR
that adds, removes, or reshapes an `env.events().publish(...)` call should update the
matching row here in the same PR. See `CONTRIBUTING.md`'s **Deprecating an entrypoint**
section for how this interacts with a planned removal.

---

## Invoice contract

Source: `contracts/invoice/src/events.rs` (the sole place invoice events are published;
all entrypoints in `contracts/invoice/src/entrypoints/` call through these helpers).

| Event | Topics | Data type | Emitted by |
|---|---|---|---|
| `invoice_created` | `(Symbol, id: u64)` | `Invoice` | `create_invoice`, `batch_create_invoice` |
| `invoice_paid` | `(Symbol, id: u64)` | `Invoice` | `mark_paid` |
| `invoice_expired` | `(Symbol, id: u64)` | `Invoice` | `batch_expire` |
| `invoice_cancelled` | `(Symbol, id: u64)` | `Invoice` | `cancel_invoice` |
| `invoice_refund_requested` | `(Symbol, id: u64)` | `Invoice` | `request_refund` |
| `refund_approved` | `(Symbol, id: u64)` | `Invoice` | `approve_refund` |
| `refund_rejected` | `(Symbol, id: u64)` | `Invoice` | `reject_refund` |
| `escrow_released` | `(Symbol, id: u64)` | `EscrowReleasedEvent` | `release_escrow` |
| `contract_paused` | `(Symbol,)` | `Address` (admin) | `pause` |
| `contract_unpaused` | `(Symbol,)` | `Address` (admin) | `unpause` |
| `invoice_amended` | `(Symbol, id: u64)` | `InvoiceAmountUpdatedEvent` | `amend_invoice` |
| `invoice_expiry_extended` | `(Symbol, id: u64)` | `InvoiceExpiryExtendedEvent` | `extend_expiry` |
| `refund_processed` | `(Symbol, id: u64)` | `RefundProcessedEvent` | `process_refund` |

**`Invoice`** (`contracts/invoice/src/invoice.rs`):
`id: u64`, `merchant: Address`, `amount_usdc: i128`, `gross_usdc: i128`,
`status: InvoiceStatus`, `expires_at: u64`, `paid_at: Option<u64>`,
`payer: MaybeAddress`, `metadata_hash: MaybeBytes`, `payment_link_hash: MaybeBytes`,
`merchant_nonce: u64`, `token_address: MaybeAddress`.

`InvoiceStatus`: `Pending | Paid | Expired | Cancelled | RefundRequested | Released | Refunded`.
`MaybeAddress`: `None | Some(Address)`. `MaybeBytes`: `None | Some(Bytes)` (`contracttype`
wrappers used in place of `Option<Address>`/`Option<Bytes>`, which soroban-sdk v20's
`contracttype` macro does not support directly).

**`EscrowReleasedEvent`**: `id: u64`, `merchant: Address`, `amount_usdc: i128`, `released_at: u64`.

**`InvoiceAmountUpdatedEvent`**: `id: u64`, `old_amount_usdc: i128`, `new_amount_usdc: i128`,
`old_gross_usdc: i128`, `new_gross_usdc: i128`.

**`InvoiceExpiryExtendedEvent`**: `id: u64`, `old_expires_at: u64`, `new_expires_at: u64`.

**`RefundProcessedEvent`** (#71): `id: u64`, `payer: Address`, `gross_amount: i128`,
`processing_fee: i128`, `network_fee: i128`, `net_amount: i128`, `processed_at: u64`.

Published alongside `refund_approved` by `process_refund`, and carrying the same
`NetRefund` record that is stored under `DataKey::RefundBreakdown(id)`. It exists
so an indexer can show a customer exactly what was deducted from their refund
without re-deriving the fee arithmetic: `gross_amount` is what was paid,
`processing_fee` is the merchant gateway's `fee_bps` share, `network_fee` is the
flat payout cost, and `net_amount` is the amount actually transferred.

### Reconstructing invoice status history

Indexers should treat the invoice ID (second topic) as the stream key and order by
ledger sequence + event position. `refund_rejected` reverts status from
`RefundRequested` back to `Paid`, the only event in this contract that moves status
"backwards"; `contract_paused`/`contract_unpaused` and `invoice_amended`/
`invoice_expiry_extended` do not represent an invoice status transition and carry no
invoice ID stream key beyond what's in their own data payload (the latter two still key
by ID in the second topic; the pause events have none, being contract-wide).

---

## Compliance contract

Source: `contracts/compliance/src/lib.rs` (single-file contract; no submodules).

| Event | Topics | Data type | Emitted by |
|---|---|---|---|
| `address_allowed` | `(Symbol,)` | `Address` | `bulk_allow_addresses` (per address), `allow_address`, `allow_address_with_tier` |
| `address_blocked` | `(Symbol,)` | `Address` | `bulk_block_addresses` (per address), `block_address` |
| `address_blocked_until` | `(Symbol,)` | `(Address, u64)` — `(address, unblock_at)` | `block_address_until` |
| `address_allowed_until` | `(Symbol,)` | `(Address, u64)` — `(address, expires_at)` | `allow_address_until` |
| `address_allow_expired` | `(Symbol,)` | `Address` | `sweep_expired` (per swept address) |
| `address_cleared` | `(Symbol,)` | `Address` | `clear_address` |
| `address_revoked` | `(Symbol,)` | `Address` | `revoke_allow` |
| `admin_transfer_initiated` | `(Symbol,)` | `Address` (new_admin) | `transfer_admin` |
| `admin_transferred` | `(Symbol,)` | `Address` (new_admin) | `accept_admin` |
| `compliance_paused` | `(Symbol,)` | `Address` (admin) | `pause` |
| `compliance_unpaused` | `(Symbol,)` | `Address` (admin) | `unpause` |
| `operator_set` | `(Symbol,)` | `Address` (operator) | `set_operator` |
| `tier_limit_set` | `(Symbol,)` | `(u32, i128)` — `(tier, limit)` | `set_tier_limit` |
| `jurisdiction_set` | `(Symbol,)` | `(Address, Bytes)` — `(address, code)` | `set_jurisdiction` |

None of these events carry the address in a second topic — indexers must decode the
data payload (or, for the tuple-payload events, its first element) to key by address.

### Tier and jurisdiction audit trail (#750)

`set_tier_limit` and `set_jurisdiction` mutate compliance-relevant configuration, so
each emits an event on every successful call to keep the audit trail complete:

- `tier_limit_set` — data `(tier: u32, limit: i128)`. Emitted whenever an admin sets
  or overwrites the per-tier transfer limit for `tier`. Indexers tracking limit
  history should order by ledger sequence + event position and treat the latest
  `tier_limit_set` for a given `tier` as the effective limit.
- `jurisdiction_set` — data `(address: Address, code: Bytes)`. Emitted whenever an
  admin assigns or overwrites the jurisdiction code for `address`. The `code` is the
  raw jurisdiction bytes as stored; the latest `jurisdiction_set` for a given
  `address` is the effective jurisdiction.

Both events are contract-wide (no second topic), so indexers must decode the data
payload to key by `tier` or `address` respectively.

`AddressState` (returned by `export_snapshot`/`export_snapshot_page`, not itself
published as an event): `Allowed | Blocked | Expired`.

---

## Settlement Workflow contract

Source: `contracts/settlement-workflow/src/lib.rs`.

| Event | Topics | Data type | Emitted by |
|---|---|---|---|
| `workflow_initialized` | `(Symbol,)` | `(Address, Address, Address)` — `(admin, compliance_id, treasury_id)` | `initialize` |
| `settlement_workflow_executed` | `(Symbol, settlement_id: u64)` | `(Address, Address)` — `(merchant, token_contract)` | `execute_with_compliance`, `execute_with_compliance_batch` (per settlement actually executed) |
| `emergency_pause_configured` | `(Symbol,)` | `(Address, Vec<Address>)` — `(admin, targets)` | `initialize_emergency_pause` |
| `emergency_pause_completed` | `(Symbol,)` | `(Address, Vec<Address>)` — `(admin, paused)` | `emergency_pause_all` |
| `emergency_pause_resumed` | `(Symbol,)` | `Vec<Address>` — `(resumed)` | `resume_all` |

### Emergency pause coordination (#73)

`emergency_pause_completed` is emitted **only after every target has confirmed
the pause**, and `emergency_pause_resumed` only after every target has confirmed
the unpause. Neither eve

/* … truncated 8346 chars — edit only what you need near the top … */
