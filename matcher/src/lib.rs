//! Blindbook matcher: the module OutLayer runs inside an Intel TDX enclave.
//!
//! It reads one JSON input from the pool contract (PROTOCOL.md §3.1), keeps
//! the resting book in the pool's storage cell, and answers with one signed
//! result the pool verifies before acting on it (§3.2).
//!
//! All logic lives behind the [`Host`] trait so it runs and is tested natively;
//! `main.rs` wires the real OutLayer host for `wasm32-wasip2`.
//!
//! # Output and its signature
//!
//! The module prints exactly one JSON object:
//!
//! ```json
//! { "result": "<canonical JSON string>", "sig": "ed25519:<base58>", "key": "ed25519:<base58>" }
//! ```
//!
//! - `result` is the compact `serde_json` serialization of the result object,
//!   with fields in the order this crate declares them.
//! - `key` is the host signing key at path `"result"` (`caller: "predecessor"`).
//! - `sig` is the 64-byte ed25519 signature (RFC 8032, no prehash, no prefix)
//!   by that key over **the 32 bytes `sha256(result)`**, where `result` is
//!   taken as the UTF-8 bytes of the string exactly as it appears in the
//!   output, before any JSON escaping. The pool checks it with
//!   `env::ed25519_verify(&sig, &env::sha256_array(result.as_bytes()), &pinned_key)`.
//!
//! When the result cannot be signed at all the module prints
//! `{"error":"<code>"}`, which the pool must treat as a failed batch.
//!
//! # Failing closed
//!
//! Any internal failure (bad input, storage, keys, a book that does not
//! decode or changed underneath the run) yields a signed result with an
//! `error` code and nothing ingested, rejected or settled, and the stored book
//! is left untouched. A remote fact that could not be fetched never fails the
//! run; it only keeps the lots that depend on it out of this batch.
//!
//! Nothing the module prints or writes to stderr contains order contents.

pub mod batch;
pub mod book;
pub mod host;
pub mod ingest;
pub mod input;
pub mod nonce;
pub mod oneclick;
pub mod rpc;
pub mod util;

use blindbook_core::envelope::MatcherKey;
use serde::Serialize;

pub use host::{Host, HostError, HttpRequest, HttpResponse, KeyId, Nep413Signed};

use crate::batch::BatchResult;
use crate::book::BOOK_KEY;
use crate::input::{BatchInput, Envelope, Op};
use crate::util::{ed25519_str, hex, sha256};

/// The network this build serves. Inputs for another network are refused.
pub const NETWORK: &str = if cfg!(feature = "mainnet") {
    "mainnet"
} else {
    "testnet"
};

/// The only account the embedded manifest admits as caller (absent from a
/// `scratch` build's manifest).
pub const POOL_ACCOUNT: &str = env!("BLINDBOOK_POOL_ACCOUNT");

/// The manifest embedded in the wasm's `outlayer.manifest` section.
pub const MANIFEST: &str = include_str!(concat!(env!("OUT_DIR"), "/manifest.json"));

/// Context of the X25519 seed MAC under the `book` encryption key.
pub const X25519_SEED_CONTEXT: &[u8] = b"blindbook/v1/matcher-x25519";

/// The `result` of `op: "keys"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeysResult {
    pub v: u32,
    pub op: &'static str,
    /// Hex of the X25519 public key orders are sealed to.
    pub matcher_x25519: String,
    /// The venue's verifier key, to register with `add_public_key`.
    pub venue: String,
    /// The key that signs every result, to pin in the pool.
    pub result_key: String,
}

#[derive(Serialize)]
struct ErrorResult {
    v: u32,
    op: &'static str,
    error: &'static str,
}

#[derive(Serialize)]
struct Signed<'a> {
    result: &'a str,
    sig: String,
    key: String,
}

/// Sign `result` with the result key and wrap it for stdout.
fn signed<H: Host>(host: &mut H, result: &str) -> Result<String, &'static str> {
    let key = host.public_key(KeyId::Result).map_err(|_| "result_key_unavailable")?;
    let sig = host
        .sign_result(&sha256(result.as_bytes()))
        .map_err(|_| "result_key_unavailable")?;
    serde_json::to_string(&Signed {
        result,
        sig: ed25519_str(&sig),
        key: ed25519_str(&key),
    })
    .map_err(|_| "result_key_unavailable")
}

fn unsigned_error(code: &'static str) -> String {
    serde_json::json!({ "error": code }).to_string()
}

fn emit<H: Host, T: Serialize>(host: &mut H, result: &T) -> String {
    match serde_json::to_string(result) {
        Ok(s) => signed(host, &s).unwrap_or_else(unsigned_error),
        Err(_) => unsigned_error("internal"),
    }
}

/// The public keys of this deployment.
pub fn keys<H: Host>(host: &mut H) -> Result<KeysResult, &'static str> {
    let seed = host.x25519_seed().map_err(|_| "key_unavailable")?;
    let venue = host.public_key(KeyId::Venue).map_err(|_| "key_unavailable")?;
    let result = host.public_key(KeyId::Result).map_err(|_| "key_unavailable")?;
    Ok(KeysResult {
        v: 1,
        op: "keys",
        matcher_x25519: hex(&MatcherKey::from_seed(seed).public_bytes()),
        venue: ed25519_str(&venue),
        result_key: ed25519_str(&result),
    })
}

/// Run one input end to end and return what the module prints.
pub fn run<H: Host>(host: &mut H, input: &[u8]) -> String {
    let input_sha256 = hex(&sha256(input));
    let envelope: Envelope = match serde_json::from_slice(input) {
        Ok(e) => e,
        Err(_) => {
            return emit(
                host,
                &ErrorResult {
                    v: 1,
                    op: "unknown",
                    error: "bad_input",
                },
            )
        }
    };
    match envelope.op {
        Op::Keys if envelope.v != 1 => emit(
            host,
            &ErrorResult {
                v: 1,
                op: "keys",
                error: "bad_input",
            },
        ),
        Op::Batch if envelope.v != 1 => emit(host, &BatchResult::failed("bad_input", input_sha256, 0)),
        Op::Keys => match keys(host) {
            Ok(k) => emit(host, &k),
            Err(code) => emit(
                host,
                &ErrorResult {
                    v: 1,
                    op: "keys",
                    error: code,
                },
            ),
        },
        Op::Batch => run_batch(host, input, input_sha256),
    }
}

fn run_batch<H: Host>(host: &mut H, input: &[u8], input_sha256: String) -> String {
    let parsed: BatchInput = match serde_json::from_slice(input) {
        Ok(p) => p,
        Err(_) => return emit(host, &BatchResult::failed("bad_input", input_sha256, 0)),
    };
    if let Err(code) = parsed.validate(NETWORK) {
        return emit(host, &BatchResult::failed(code, input_sha256, parsed.now_ns));
    }
    let outcome = match batch::run(host, &parsed, input_sha256.clone()) {
        Ok(o) => o,
        Err(code) => return emit(host, &BatchResult::failed(code, input_sha256, parsed.now_ns)),
    };
    let result = match serde_json::to_string(&outcome.result) {
        Ok(s) => s,
        Err(_) => return emit(host, &BatchResult::failed("internal", input_sha256, parsed.now_ns)),
    };
    // Sign before storing: a result that cannot be signed must not move the book.
    let out = match signed(host, &result) {
        Ok(out) => out,
        Err(code) => return unsigned_error(code),
    };
    match host.storage_put(BOOK_KEY, outcome.stored.as_deref(), &outcome.book_bytes) {
        Ok(true) => out,
        Ok(false) => emit(host, &BatchResult::failed("book_conflict", input_sha256, parsed.now_ns)),
        Err(_) => emit(
            host,
            &BatchResult::failed("storage_unavailable", input_sha256, parsed.now_ns),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn manifest_declares_keys_storage_callers_and_network() {
        let m: Value = serde_json::from_str(MANIFEST).unwrap();
        let keys = m["signing_keys"].as_array().unwrap();
        assert_eq!(keys.len(), 2);
        for (k, path) in keys.iter().zip(["venue", "result"]) {
            assert_eq!(k["path"], path);
            assert_eq!(k["type"], "ed25519");
            assert_eq!(k["caller"], "predecessor");
        }
        assert_eq!(m["encryption_keys"][0]["path"], "book");
        assert_eq!(m["encryption_keys"][0]["caller"], "predecessor");
        assert_eq!(m["storage_account"], "predecessor");
        let net: Vec<&str> = m["capabilities"]["network"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap())
            .collect();
        assert_eq!(
            net,
            [
                "1click.chaindefuser.com",
                "rpc.mainnet.fastnear.com",
                "test.rpc.fastnear.com"
            ]
        );
        if cfg!(feature = "scratch") {
            assert!(m.get("callers").is_none());
        } else {
            assert_eq!(m["callers"]["contract"]["only"], serde_json::json!([POOL_ACCOUNT]));
        }
        let allowed = [
            "signing_keys",
            "encryption_keys",
            "storage_account",
            "callers",
            "capabilities",
        ];
        assert!(m.as_object().unwrap().keys().all(|k| allowed.contains(&k.as_str())));
    }

    #[test]
    fn default_build_is_testnet() {
        if !cfg!(feature = "mainnet") {
            assert_eq!(NETWORK, "testnet");
            if std::option_env!("BLINDBOOK_POOL").is_none() {
                assert_eq!(POOL_ACCOUNT, "pool.blindbook-dev.testnet");
            }
        }
    }
}
