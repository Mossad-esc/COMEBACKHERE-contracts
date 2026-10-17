#![no_main]

// Fuzz harness (cargo-fuzz) for refund.rs::calculate_net_refund, which mixes
// i128 and bps math. Unit tests in refund.rs cover fixed cases only; this
// harness throws arbitrary i128/i128 inputs at it to assert the invariants
// that matter for any input that is accepted:
//   - net + fee == gross
//   - net is never negative
//   - the function never panics (overflow, divide-by-zero, etc.)

use arbitrary::Arbitrary;
use invoice::calculate_net_refund;
use libfuzzer_sys::fuzz_target;

#[derive(Debug, Arbitrary)]
struct Input {
    gross_usdc: i128,
    fee_bps: i128,
}

fuzz_target!(|input: Input| {
    if let Ok((net, fee)) = calculate_net_refund(input.gross_usdc, input.fee_bps) {
        assert_eq!(net + fee, input.gross_usdc, "net + fee must equal gross");
        assert!(net >= 0, "net must never be negative");
        assert!(fee >= 0, "fee must never be negative");
    }
    // Err is acceptable for out-of-range inputs; a panic is not.
});
