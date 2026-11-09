//! Reference prices from signed 1Click dry quotes.

use blindbook_core::intent::parse_rfc3339_ns;
use blindbook_core::{verify_quote, Price, VerifiedQuote, ONE_CLICK_MANAGER_PUB_KEY};
use serde_json::{json, Value};

use crate::host::{HostError, HttpRequest, HttpResponse};
use crate::util::{rfc3339, NS_PER_S};

pub const QUOTE_URL: &str = "https://1click.chaindefuser.com/v0/quote";

/// Account named as recipient and refund address of the dry quotes. A dry
/// quote moves nothing; 1Click only needs a well-formed account.
pub const DRY_QUOTE_ACCOUNT: &str = "intents.near";

/// Why a market has no usable reference this batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteFail {
    /// No answer, an error status, or a body that is not a quote.
    Unavailable,
    /// The signature does not verify against the 1Click key, or the signed
    /// request is not the one we asked for.
    Unverified,
    /// Older (or further in the future) than the freshness window allows.
    Stale,
}

/// A dry `EXACT_INPUT` quote request selling `amount` of `origin` for `destination`.
pub fn request(origin: &str, destination: &str, amount: u128, now_ns: u64) -> HttpRequest {
    let body = json!({
        "dry": true,
        "swapType": "EXACT_INPUT",
        "slippageTolerance": 100,
        "originAsset": origin,
        "depositType": "INTENTS",
        "destinationAsset": destination,
        "amount": amount.to_string(),
        "refundTo": DRY_QUOTE_ACCOUNT,
        "refundType": "INTENTS",
        "recipient": DRY_QUOTE_ACCOUNT,
        "recipientType": "INTENTS",
        "deadline": rfc3339(now_ns / NS_PER_S + 1_800),
    });
    HttpRequest {
        url: QUOTE_URL.to_string(),
        body: body.to_string().into_bytes(),
    }
}

/// Check a quote response: signed by the 1Click key, for exactly the request
/// we made (dry, exact input, these assets, this amount) and fresh.
/// `max_age_s = None` disables the freshness check.
pub fn verify(
    resp: &Result<HttpResponse, HostError>,
    origin: &str,
    destination: &str,
    amount: u128,
    now_ns: u64,
    max_age_s: Option<u64>,
) -> Result<VerifiedQuote, QuoteFail> {
    let resp = resp.as_ref().map_err(|_| QuoteFail::Unavailable)?;
    if !(200..300).contains(&resp.status) {
        return Err(QuoteFail::Unavailable);
    }
    let v: Value = serde_json::from_slice(&resp.body).map_err(|_| QuoteFail::Unavailable)?;
    let q = verify_quote(&v, ONE_CLICK_MANAGER_PUB_KEY).map_err(|_| QuoteFail::Unverified)?;
    let req = &v["quoteRequest"];
    let as_requested = req["dry"] == Value::Bool(true)
        && req["swapType"] == "EXACT_INPUT"
        && q.origin_asset == origin
        && q.destination_asset == destination
        && req["amount"].as_str() == Some(amount.to_string().as_str())
        && q.amount_in == amount
        && q.amount_out > 0;
    if !as_requested {
        return Err(QuoteFail::Unverified);
    }
    if let Some(max_age) = max_age_s {
        let ts = parse_rfc3339_ns(&q.timestamp).ok_or(QuoteFail::Stale)?;
        if now_ns.abs_diff(ts) > max_age.saturating_mul(NS_PER_S) {
            return Err(QuoteFail::Stale);
        }
    }
    Ok(q)
}

/// Bid and ask of `base` in `quote` from a verified sell-base quote and a
/// verified sell-quote quote.
pub fn bid_ask(bid: &VerifiedQuote, ask: &VerifiedQuote, base: &str, quote: &str) -> Result<(Price, Price), QuoteFail> {
    let b = bid.price_for(base, quote).map_err(|_| QuoteFail::Unverified)?;
    let a = ask.price_for(base, quote).map_err(|_| QuoteFail::Unverified)?;
    Ok((b, a))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WNEAR: &str = "nep141:wrap.near";
    const USDC: &str = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
    const PROBE_BASE: u128 = 10_000_000_000_000_000_000_000_000;
    const PROBE_QUOTE: u128 = 36_000_000;
    /// 2026-11-09T12:29:30Z, about 20 s after the fixtures were signed.
    const NOW: u64 = 1_794_227_370_000_000_000;

    fn fixture(name: &str) -> Result<HttpResponse, HostError> {
        let path = format!(
            "{}/../crates/core/tests/fixtures/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        Ok(HttpResponse {
            status: 201,
            body: std::fs::read(path).unwrap(),
        })
    }

    #[test]
    fn request_is_a_dry_exact_input_quote() {
        let r = request(WNEAR, USDC, PROBE_BASE, NOW);
        let v: Value = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(r.url, QUOTE_URL);
        assert_eq!(v["dry"], true);
        assert_eq!(v["swapType"], "EXACT_INPUT");
        assert_eq!(v["amount"], PROBE_BASE.to_string());
        assert_eq!(v["deadline"], "2026-11-09T12:59:30.000Z");
    }

    #[test]
    fn fixtures_give_a_bid_below_the_ask() {
        let bid = verify(&fixture("quote_sell"), WNEAR, USDC, PROBE_BASE, NOW, Some(120)).unwrap();
        let ask = verify(&fixture("quote_buy"), USDC, WNEAR, PROBE_QUOTE, NOW, Some(120)).unwrap();
        let (b, a) = bid_ask(&bid, &ask, WNEAR, USDC).unwrap();
        assert!(b.cmp_ratio(&a).is_lt());
        assert!(bid.digest_b58.len() > 40);
    }

    #[test]
    fn stale_or_future_quotes_are_refused() {
        let late = NOW + 600 * NS_PER_S;
        assert_eq!(
            verify(&fixture("quote_sell"), WNEAR, USDC, PROBE_BASE, late, Some(120)),
            Err(QuoteFail::Stale)
        );
        let early = NOW - 600 * NS_PER_S;
        assert_eq!(
            verify(&fixture("quote_sell"), WNEAR, USDC, PROBE_BASE, early, Some(120)),
            Err(QuoteFail::Stale)
        );
        assert!(verify(&fixture("quote_sell"), WNEAR, USDC, PROBE_BASE, late, None).is_ok());
    }

    #[test]
    fn a_quote_for_something_else_is_refused() {
        // another amount
        assert_eq!(
            verify(&fixture("quote_sell"), WNEAR, USDC, PROBE_BASE + 1, NOW, None),
            Err(QuoteFail::Unverified)
        );
        // the other direction
        assert_eq!(
            verify(&fixture("quote_sell"), USDC, WNEAR, PROBE_BASE, NOW, None),
            Err(QuoteFail::Unverified)
        );
    }

    #[test]
    fn tampered_and_failed_responses() {
        let mut r = fixture("quote_sell").unwrap();
        let s = String::from_utf8(r.body.clone())
            .unwrap()
            .replace("\"35309712\"", "\"35309713\"");
        r.body = s.into_bytes();
        assert_eq!(
            verify(&Ok(r), WNEAR, USDC, PROBE_BASE, NOW, None),
            Err(QuoteFail::Unverified)
        );
        let e = Ok(HttpResponse {
            status: 500,
            body: b"{}".to_vec(),
        });
        assert_eq!(
            verify(&e, WNEAR, USDC, PROBE_BASE, NOW, None),
            Err(QuoteFail::Unavailable)
        );
        assert_eq!(
            verify(&Err(HostError::new("x")), WNEAR, USDC, PROBE_BASE, NOW, None),
            Err(QuoteFail::Unavailable)
        );
        let garbage = Ok(HttpResponse {
            status: 200,
            body: b"<html>".to_vec(),
        });
        assert_eq!(
            verify(&garbage, WNEAR, USDC, PROBE_BASE, NOW, None),
            Err(QuoteFail::Unavailable)
        );
    }
}
