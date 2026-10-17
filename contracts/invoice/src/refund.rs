use crate::InvoiceError;

/// Basis-point denominator (100.00% == 10_000 bps).
const BPS_DENOMINATOR: i128 = 10_000;

/// Splits a gross refund amount into `(net, fee)` given a fee in basis
/// points, such that `net + fee == gross` and `net` is never negative.
///
/// `gross_usdc` must be non-negative and `fee_bps` must be in `[0, 10_000]`;
/// otherwise this returns `Err` rather than panicking.
pub fn calculate_net_refund(
    gross_usdc: i128,
    fee_bps: i128,
) -> Result<(i128, i128), InvoiceError> {
    if gross_usdc < 0 {
        return Err(InvoiceError::InvalidAmount);
    }
    if !(0..=BPS_DENOMINATOR).contains(&fee_bps) {
        return Err(InvoiceError::InvalidFeeBps);
    }

    let fee = gross_usdc
        .checked_mul(fee_bps)
        .ok_or(InvoiceError::InvalidAmount)?
        .checked_div(BPS_DENOMINATOR)
        .ok_or(InvoiceError::InvalidAmount)?;

    let net = gross_usdc
        .checked_sub(fee)
        .ok_or(InvoiceError::InvalidAmount)?;

    Ok((net, fee))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_gross_exactly() {
        let (net, fee) = calculate_net_refund(1_000_000, 250).unwrap();
        assert_eq!(net + fee, 1_000_000);
        assert!(net >= 0);
    }

    #[test]
    fn zero_fee_returns_full_gross_as_net() {
        let (net, fee) = calculate_net_refund(1_000_000, 0).unwrap();
        assert_eq!(net, 1_000_000);
        assert_eq!(fee, 0);
    }

    #[test]
    fn full_fee_returns_zero_net() {
        let (net, fee) = calculate_net_refund(1_000_000, 10_000).unwrap();
        assert_eq!(net, 0);
        assert_eq!(fee, 1_000_000);
    }

    #[test]
    fn rejects_negative_gross() {
        assert!(calculate_net_refund(-1, 100).is_err());
    }

    #[test]
    fn rejects_out_of_range_fee_bps() {
        assert!(calculate_net_refund(1_000_000, 10_001).is_err());
        assert!(calculate_net_refund(1_000_000, -1).is_err());
    }
}
