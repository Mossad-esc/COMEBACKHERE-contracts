# Event schema versioning

This document tracks versioned changes to contract event payloads so
off-chain indexers can detect and handle schema changes.

## `invoice_created`

### v2 (current)

Payload: `InvoiceCreatedEvent`

```
InvoiceCreatedEvent {
    id: u64,
    merchant: Address,
    amount_usdc: i128,
    gross_usdc: i128,
    expires_at: u64,
    merchant_nonce: u64,
    token_address: MaybeAddress,
}
```

`token_address` is `MaybeAddress::None` for the default USDC-denominated
invoice, or `MaybeAddress::Some(address)` for a multi-currency invoice.
Added so indexers can learn an invoice's settlement asset directly from
the creation event instead of re-fetching the invoice via `get_invoice`.

### v1 (previous)

Payload: full `Invoice` struct, published verbatim.

Superseded by v2 because consumers only needed a subset of invoice
fields, and the full struct did not make the settlement token
obviously discoverable to webhook/indexer consumers that only inspect
event topics and payload fields they know about.
