# Storage TTL Audit

Tracks which entrypoints extend the TTL of persistent storage entries they
touch, so invoices do not archive early.

## Invoice Contract

| Entrypoint | Persistent key(s) touched | TTL bump applied |
|---|---|---|
| `batch_create_invoice` | `DataKey::Invoice(id)` | Yes — `extend_ttl(INVOICE_TTL_THRESHOLD, INVOICE_TTL_EXTEND_TO)` after each write |
| `batch_expire` | `DataKey::Invoice(id)` | Yes — `extend_ttl(INVOICE_TTL_THRESHOLD, INVOICE_TTL_EXTEND_TO)` after each transition to `Expired` |

## Constants

- `INVOICE_TTL_THRESHOLD = 518_400` ledgers (~30 days at 5s/ledger) — the
  remaining-TTL floor below which an entry is eligible for extension.
- `INVOICE_TTL_EXTEND_TO = 1_036_800` ledgers (~60 days at 5s/ledger) — the
  TTL an entry is extended to once bumped.

Both constants live in `contracts/invoice/src/invoice.rs` and are re-exported
from `contracts/invoice/src/lib.rs` so entrypoint modules share a single
source of truth.
