# Recurring invoice templates, keeper, signer activity and late-fee event

Closes #692
Closes #698
Closes #699
Closes #704

## What changed

### #692 — `SignerLastActive` storage key (treasury)

`record_approval` had no key to write to, so signer activity was never tracked.

- Appended `DataKey::SignerLastActive(Address)` to the end of the enum, so existing
  storage layouts are untouched.
- `settlement.rs` gained `record_approval` (stamps the current ledger timestamp) and
  `signer_last_active` (reads the same key back).
- Every approval path now records activity: `approve_settlement`,
  `approve_partial_settlement`, `vote_dispute_resolution`,
  `approve_signer_rotation`.
- New read entrypoint `get_signer_last_active(signer) -> u64` (0 = never approved).
- Tests: `contracts/treasury/tests/signer_activity_test.rs` (8 tests: never-approved,
  settlement/partial/dispute/rotation approvals, refresh on a later approval, duplicate
  approval does not refresh, per-signer isolation).

### #698 — template read entrypoints (invoice)

Templates had no read path at all. Entrypoints are now split into an `entrypoints`
module (`lifecycle.rs`, `template.rs`) that the single `#[contractimpl]` delegates to.

- `get_invoice_template(id)` — single read, `NotFound` for unknown ids.
- `list_templates_by_merchant(merchant, start, limit)` — per-merchant page in creation
  order, `limit` capped at `MAX_PAGE_LIMIT` (50).
- Supporting writes: `create_template`, `set_template_active`, and a
  `MerchantTemplates(Address)` index.
- Tests: `contracts/invoice/tests/invoice_template_read_test.rs` (14 tests: empty, single,
  multi-page, start past the end, limit cap, per-merchant scoping, validation, activation).

### #699 — keeper entrypoint for due templates (invoice)

- `InvoiceTemplate` carries `next_run_at`; the first run is scheduled one full interval
  after creation so a new template is never immediately due.
- `generate_due_invoices(template_ids)` — permissionless, bounded by `MAX_BATCH_SIZE`
  (20, otherwise `BatchTooLarge`), skips unknown/inactive/not-yet-due templates, emits
  `template_generated` per invoice and returns the number generated.
- Periods cannot be invoiced twice: each generated invoice advances `next_run_at` past the
  current timestamp in the same call, so a second call in the same period is a no-op and a
  late keeper call backfills one invoice rather than a backlog.
- Tests: `contracts/invoice/tests/invoice_template_keeper_test.rs` (13 tests: not-yet-due,
  due, double-call in the same period, consecutive periods, long gap, batch, unknown ids,
  deactivated template, batch-size bounds, empty batch, pause, generated invoice is an
  ordinary invoice).

### #704 — late fee event (invoice)

- `set_late_fee_bps(admin, bps)` stores the fee (rejecting values above 10 000 bps with
  `InvalidLateFee`), `get_late_fee_bps()` reads it, and the update emits
  `late_fee_updated` with `(old_bps, new_bps)`.
- Tests: `contracts/invoice/tests/invoice_late_fee_test.rs` (6 tests, including the event
  snapshot of old/new bps).
- New `docs/event-schema.md` documents every invoice, treasury and compliance event.

### Baseline repairs (needed to get the gates running)

`main` did not compile, so nothing above could be verified or merged:

- `crates/compliance-client`: `compliance` was a dev-dependency but re-exported from `lib.rs`.
- `contracts/invoice`: test call sites updated to the 7-argument `create_invoice`, a
  535-line orphaned duplicate block removed, the interleaved escrow/cancel tests in
  `invoice_test.rs` rebuilt, and the ABI snapshot assertion now skips when
  `abis/invoice.json` is absent (contracts-only checkout).
- `contracts/compliance`: restored the missing `last_event_symbol` helper, added the
  `reason` argument to `block_address`, and dropped the `export_snapshot` tests — that
  entrypoint is not part of this contract revision and the block mixed 1-arg and 3-arg
  calls, so it could never compile.
- `tests`: fixed the unresolved module path for the inline workflow contract.
- `create_invoice` now guards the id counter with `checked_add` and returns
  `InvoiceIdOverflow` instead of silently wrapping to id 0 in release WASM.
- WASM builds: the contracts declare `default = ["testutils"]`, which fails on
  `wasm32-unknown-unknown`. The CI build and size workflows now pass `--no-default-features`
  (also in the `justfile` deploy recipe).
- Contract size: the treasury sat 400 bytes under the 64 KiB cap, so the release profile
  is optimised and stripped.

## Verification

```
cargo fmt --all -- --check                                  # clean
cargo clippy --workspace --all-targets -- -D warnings       # clean
cargo build --target wasm32-unknown-unknown --release --no-default-features -p <each contract>
```

WASM sizes (limit 65 536): compliance 27 958 · invoice 43 036 · treasury 53 647.

Tests: invoice 96 passed · compliance 49 passed · treasury 14 passed in the binaries that
run to completion, including the new `signer_activity_test` (8), `invoice_template_read_test` (14),
`invoice_template_keeper_test` (13) and `invoice_late_fee_test` (6).

## Known pre-existing failure (unchanged by this PR)

Seven treasury test binaries and `tests/full_lifecycle_smoke_test.rs` abort the test process
before this PR and still do: a contract-side `panic!` becomes a non-unwinding panic under
soroban-sdk 20.5 + rustc 1.95, so every `#[should_panic(expected = "...")]` test that
relies on a contract panic kills its binary (`partial_settlement_test`,
`pause_blocks_treasury_test`, `task1_tests`, `threshold_update_test`,
`treasury_rotation_executed_rejection_test`, `treasury_threshold_rejection_test`,
`treasury_token_allowlist_test`, `unauthorized_signer_test`). Reproduced on `main` with a
minimal two-line contract that only calls `panic!`. Fixing it means moving the treasury off
panic-string errors to `Result` returns, which is an ABI change beyond these issues; it
needs its own issue.