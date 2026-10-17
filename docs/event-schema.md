# Event Schema

All contracts emit events through the Soroban SDK, so every event has

| Field  | Meaning |
|--------|---------|
| topics | `[Symbol, ...]` — topic 0 is always the event name, remaining topics are identifiers |
| data   | One serializable value (struct, enum, tuple or primitive) |

Values serialize to JSON for Redis/webhook delivery: symbols and enum variants
become strings, `Address` becomes an account id, integers become JSON numbers
or strings, structs become objects keyed by field name, and `Option<T>` becomes
`null` or the inner value.

---

## Invoice contract (`comebackhere-invoice`)

| Event | Topics | Data |
|-------|--------|------|
| `invoice_created` | `[invoice_created, id]` | `Invoice` |
| `invoice_paid` | `[invoice_paid, id]` | `Invoice` |
| `invoice_expired` | `[invoice_expired, id]` | `Invoice` |
| `invoice_cancelled` | `[invoice_cancelled, id]` | `Invoice` |
| `invoice_refund_req` | `[invoice_refund_req, id]` | `Invoice` |
| `escrow_released` | `[escrow_released, id]` | `Invoice` |
| `contract_paused` | `[contract_paused]` | `Address` (admin) |
| `contract_unpaused` | `[contract_unpaused]` | `Address` (admin) |
| `template_created` | `[template_created, id]` | `InvoiceTemplate` |
| `template_activated` | `[template_activated, id]` | `bool` (new `active` flag) |
| `template_generated` | `[template_generated, template_id, invoice_id]` | none |
| `late_fee_updated` | `[late_fee_updated]` | `(old_bps, new_bps)` |

### `template_created`

Emitted when a merchant registers a recurring invoice template. `next_run_at`
is one full interval after the creation timestamp, so a fresh template is never
due immediately.

```json
{
  "id": 1,
  "merchant": "G...",
  "amount_usdc": 25000000,
  "gross_usdc": 25750000,
  "interval": "weekly",
  "expires_in_seconds": 3600,
  "metadata_hash": null,
  "payment_link_hash": null,
  "active": true,
  "next_run_at": 172800
}
```

### `template_generated`

Emitted once per invoice the keeper creates from a due template. Topic 1 is the
template id, topic 2 is the new invoice id, so an indexer can join the event
back to `invoice_created`. A period can never be invoiced twice: the keeper
advances `next_run_at` past the current ledger timestamp in the same call.

### `late_fee_updated`

Emitted by `set_late_fee_bps` whenever the admin changes the late fee. Late
fees change what a late payer owes, so indexers must be able to see the change.
`old_bps` is the value in force before the update (0 when the fee was never
configured) and `new_bps` is the value now stored, in basis points, capped at
10 000 (100%).

```json
[250, 500]
```

---

## Treasury contract (`comebackhere-treasury`)

Events are published inline by each entrypoint: topics are
`[event_name, id]` (or `[event_name, address]`) and data is the affected
struct, or the primitive field that changed.

| Event | Topics | Data |
|-------|--------|------|
| `treasury_initialized` | `[treasury_initialized]` | `Address` (admin) |
| `signer_weight_set` | `[signer_weight_set, signer]` | `u32` weight |
| `settlement_proposed` | `[settlement_proposed, id]` | `Settlement` |
| `settlement_approved` | `[settlement_approved, id]` | `Settlement` |
| `settlement_partial_approved` | `[settlement_partial_approved, id]` | `Settlement` |
| `settlement_executed` | `[settlement_executed, id]` | `Settlement` |
| `settlement_partial_executed` | `[settlement_partial_executed, id]` | `Settlement` |
| `settlement_cancelled` | `[settlement_cancelled, id]` | `Settlement` |
| `settlement_held` | `[settlement_held, id]` | `SettlementHoldReason` |
| `settlement_released` | `[settlement_released, id]` | `Settlement` |
| `threshold_updated` | `[threshold_updated]` | `u32` threshold |
| `treasury_paused` / `treasury_unpaused` | `[treasury_paused]` / `[treasury_unpaused]` | `Address` (admin) |
| `dispute_raised` | `[dispute_raised, id]` | `Dispute` |
| `dispute_resolved` | `[dispute_resolved, id]` | `Dispute` |
| `dispute_resolution_voted` | `[dispute_resolution_voted, id]` | `Dispute` |
| `deposit` / `withdraw` | `[deposit, from]` / `[withdraw, to]` | `i128` amount |
| `treasury_drained` | `[treasury_drained]` | `Address` (recipient) |
| `token_allowed` / `token_removed` | `[token_allowed]` / `[token_removed]` | `Address` token |
| `rotation_proposed` / `rotation_approved` / `rotation_executed` | `[rotation_*, id]` | `SignerRotationProposal` |
| `merchant_payout_updated` | `[merchant_payout_updated, merchant]` | `Address` payout address |

Signer activity is not an event: read `get_signer_last_active(signer)` for the
ledger timestamp of a signer's most recent recorded approval (0 when the signer
has never approved).

---

## Compliance contract (`comebackhere-compliance`)

| Event | Topics | Data |
|-------|--------|------|
| `address_allowed` | `[address_allowed]` | `Address` |
| `address_allowed_until` | `[address_allowed_until]` | `(Address, u64 expires_at)` |
| `address_blocked` | `[address_blocked]` | `Address` |
| `address_blocked_until` | `[address_blocked_until]` | `(Address, u64 expires_at)` |
| `address_cleared` | `[address_cleared]` | `Address` |
| `address_revoked` | `[address_revoked]` | `Address` |
| `compliance_paused` / `compliance_unpaused` | `[compliance_paused]` / `[compliance_unpaused]` | `Address` (admin) |
| `admin_transfer_initiated` / `admin_transferred` | `[admin_transfer_*]` | `Address` (new admin) |
| `operator_set` | `[operator_set]` | `Address` (operator) |