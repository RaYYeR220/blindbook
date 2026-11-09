//! Exact price arithmetic in token base units.
//!
//! A [`Price`] is a ratio `quote / base` of two integer amounts in smallest
//! units (e.g. 3_600_000 µUSDC per 10^24 yoctoNEAR). All conversions use
//! 256-bit intermediates, so a 10^30 base amount times a 10^12 quote amount
//! never overflows.

use primitive_types::U256;
use serde::{Deserialize, Serialize};

use crate::Amount;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    /// Quote units received for `base` units.
    #[serde(with = "crate::u128_str")]
    pub quote: Amount,
    /// Base units the quote amount refers to. Never zero.
    #[serde(with = "crate::u128_str")]
    pub base: Amount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PriceError {
    #[error("price has a zero base amount")]
    ZeroBase,
    #[error("bid {bid_quote}/{bid_base} is above ask {ask_quote}/{ask_base}")]
    Crossed {
        bid_quote: Amount,
        bid_base: Amount,
        ask_quote: Amount,
        ask_base: Amount,
    },
    #[error("spread of {spread_bps} bps exceeds the {max_bps} bps limit")]
    SpreadTooWide { spread_bps: u32, max_bps: u32 },
    #[error("arithmetic overflow")]
    Overflow,
}

impl Price {
    pub fn new(quote: Amount, base: Amount) -> Result<Self, PriceError> {
        if base == 0 {
            return Err(PriceError::ZeroBase);
        }
        Ok(Self { quote, base })
    }

    /// Quote amount for `base_amount` at this price, rounded down.
    pub fn quote_for(&self, base_amount: Amount) -> Result<Amount, PriceError> {
        mul_div_floor(base_amount, self.quote, self.base)
    }

    /// Compare `self` with `other` without division. Returns `Ordering` of the
    /// two ratios.
    pub fn cmp_ratio(&self, other: &Price) -> core::cmp::Ordering {
        let lhs = U256::from(self.quote) * U256::from(other.base);
        let rhs = U256::from(other.quote) * U256::from(self.base);
        lhs.cmp(&rhs)
    }
}

/// Midpoint of a two-sided reference market.
///
/// `bid` is what the market pays per base when you sell base; `ask` is what it
/// charges per base when you buy. The midpoint is expressed on a common
/// denominator so it stays exact: `mid = (bid + ask) / 2`.
///
/// Fails when the market is crossed or the spread is wider than
/// `max_spread_bps` of the midpoint, because a dislocated reference is the one
/// situation where crossing at "mid" would be unfair to one side.
pub fn midpoint(bid: Price, ask: Price, max_spread_bps: u32) -> Result<Price, PriceError> {
    if bid.base == 0 || ask.base == 0 {
        return Err(PriceError::ZeroBase);
    }
    if bid.cmp_ratio(&ask) == core::cmp::Ordering::Greater {
        return Err(PriceError::Crossed {
            bid_quote: bid.quote,
            bid_base: bid.base,
            ask_quote: ask.quote,
            ask_base: ask.base,
        });
    }
    // mid = (bq/bb + aq/ab) / 2 = (bq*ab + aq*bb) / (2*bb*ab)
    let bq = U256::from(bid.quote);
    let bb = U256::from(bid.base);
    let aq = U256::from(ask.quote);
    let ab = U256::from(ask.base);
    let num = bq * ab + aq * bb;
    let den = U256::from(2u8) * bb * ab;

    // spread_bps = (ask - bid) / mid * 10_000
    //            = (aq*bb - bq*ab) * 2 * 10_000 / (bq*ab + aq*bb)
    if num.is_zero() {
        return Err(PriceError::ZeroBase);
    }
    let diff = aq * bb - bq * ab;
    let spread_bps = (diff * U256::from(20_000u32)) / num;
    let spread_bps: u32 = if spread_bps > U256::from(u32::MAX) {
        u32::MAX
    } else {
        spread_bps.as_u32()
    };
    if spread_bps > max_spread_bps {
        return Err(PriceError::SpreadTooWide {
            spread_bps,
            max_bps: max_spread_bps,
        });
    }
    // Express the midpoint on the larger reference size, scaled by 10^6 for
    // sub-unit precision, rounding once: quote = floor(num * base / den).
    let reference = bid.base.max(ask.base);
    let max = U256::from(u128::MAX);
    for base in [reference.checked_mul(1_000_000), Some(reference)].into_iter().flatten() {
        let quote = num * U256::from(base) / den;
        if quote <= max {
            return Price::new(quote.as_u128(), base);
        }
    }
    Err(PriceError::Overflow)
}

/// `a * b / c`, rounded down, with a 256-bit intermediate.
pub fn mul_div_floor(a: Amount, b: Amount, c: Amount) -> Result<Amount, PriceError> {
    if c == 0 {
        return Err(PriceError::ZeroBase);
    }
    let r = U256::from(a) * U256::from(b) / U256::from(c);
    if r > U256::from(u128::MAX) {
        return Err(PriceError::Overflow);
    }
    Ok(r.as_u128())
}

/// `a * bps / 10_000`, rounded down.
pub fn bps_of(a: Amount, bps: u32) -> Result<Amount, PriceError> {
    mul_div_floor(a, bps as Amount, 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEAR: Amount = 1_000_000_000_000_000_000_000_000; // 10^24
    const USDC: Amount = 1_000_000; // 10^6

    #[test]
    fn quote_for_uses_exact_integer_math() {
        // 3.60 USDC per NEAR
        let p = Price::new(3_600_000, NEAR).unwrap();
        assert_eq!(p.quote_for(10 * NEAR).unwrap(), 36 * USDC);
        assert_eq!(p.quote_for(100_000 * NEAR).unwrap(), 360_000 * USDC);
    }

    #[test]
    fn quote_for_rounds_down() {
        // 1 quote unit per 3 base units; 10 base -> 3 quote (3.33 floored)
        let p = Price::new(1, 3).unwrap();
        assert_eq!(p.quote_for(10).unwrap(), 3);
    }

    #[test]
    fn midpoint_of_bid_and_ask() {
        let bid = Price::new(3_590_000, NEAR).unwrap();
        let ask = Price::new(3_610_000, NEAR).unwrap();
        let mid = midpoint(bid, ask, 100).unwrap();
        assert_eq!(mid.quote_for(NEAR).unwrap(), 3_600_000);
    }

    #[test]
    fn midpoint_handles_different_reference_sizes() {
        // bid quoted on 10 NEAR, ask quoted on 7 NEAR
        let bid = Price::new(35_900_000, 10 * NEAR).unwrap(); // 3.59
        let ask = Price::new(25_270_000, 7 * NEAR).unwrap(); // 3.61
        let mid = midpoint(bid, ask, 100).unwrap();
        assert_eq!(mid.quote_for(1_000 * NEAR).unwrap(), 3_600 * USDC);
    }

    #[test]
    fn midpoint_rejects_crossed_reference() {
        let bid = Price::new(3_620_000, NEAR).unwrap();
        let ask = Price::new(3_600_000, NEAR).unwrap();
        assert!(matches!(midpoint(bid, ask, 100), Err(PriceError::Crossed { .. })));
    }

    #[test]
    fn midpoint_rejects_wide_spread() {
        // 3.40 / 3.80 -> spread 400/3.60 ≈ 1111 bps
        let bid = Price::new(3_400_000, NEAR).unwrap();
        let ask = Price::new(3_800_000, NEAR).unwrap();
        let err = midpoint(bid, ask, 100).unwrap_err();
        assert!(matches!(err, PriceError::SpreadTooWide { spread_bps: 1111, max_bps: 100 }));
    }

    #[test]
    fn zero_base_is_rejected() {
        assert_eq!(Price::new(1, 0), Err(PriceError::ZeroBase));
    }

    #[test]
    fn bps_rounds_down() {
        assert_eq!(bps_of(36 * USDC, 5).unwrap(), 18_000); // 0.018 USDC
        assert_eq!(bps_of(1, 5).unwrap(), 0);
    }
}
