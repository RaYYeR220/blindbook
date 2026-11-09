//! `pool-echo`: a TEST-ONLY OutLayer module for the pool's end-to-end test.
//!
//! It speaks the matcher's I/O (docs/PROTOCOL.md §3) without matching anything:
//! - `op: "keys"` publishes a predecessor-bound host signing key as the result
//!   key, an X25519 key derived from a TEE encryption key, and a fixed venue key.
//! - `op: "batch"` opens each order as an *unencrypted* echo envelope (same
//!   layout and bucket sizes as a sealed one, plaintext in the clear) holding an
//!   instruction `{echo: 1, settle, batches, book?, reject?, forge?}`. It
//!   ingests those orders, rejects the rest, and returns the instructions'
//!   settle payloads, signed as `ed25519(sha256(result))` with the host key,
//!   echoing `input_sha256` like the real matcher (§3.4).
//!   `forge: "wrong_key"` signs with another key while claiming the host key;
//!   `forge: "tamper"` signs one result and returns another.
//!
//! Real orders are sealed; never point a production pool at this module.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ed25519_dalek::{Signer, SigningKey};
use outlayer::{encryption_keys, env, signing_keys};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use x25519_dalek::{PublicKey as XPub, StaticSecret};

#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!("../manifest.json").len()] =
    *include_bytes!("../manifest.json");

/// The venue key the e2e registered on the testnet verifier (public, from the defuse spike).
const VENUE_KEY: &str = "ed25519:2Ntcp6GVLcDa64BgCwVrzHT1LYB3vLGzp9G25DcuTTdD";
const HEADER: usize = 1 + 32 + 24;
const TAG: usize = 16;

fn main() {
    let raw = env::input();
    let input: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let out = match input["op"].as_str() {
        Some("keys") => keys(),
        Some("batch") => batch(&input, &hex::encode(sha256(&raw))),
        other => Err(format!("unknown op {other:?}")),
    };
    match out {
        Ok(v) => {
            print!("{v}");
            let _ = std::io::stdout().flush();
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

fn result_key() -> Result<String, String> {
    let pk = signing_keys::public_key("result", None).map_err(|e| e.to_string())?;
    Ok(format!("ed25519:{}", bs58::encode(pk).into_string()))
}

fn sign_host(msg: &[u8]) -> Result<Vec<u8>, String> {
    signing_keys::sign("result", None, msg).map_err(|e| e.to_string())
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

fn output(result: &str, sig: &[u8], key: &str) -> Value {
    json!({ "result": result, "sig": format!("ed25519:{}", bs58::encode(sig).into_string()), "key": key })
}

fn keys() -> Result<Value, String> {
    let mac = encryption_keys::mac("book", None, b"blindbook/echo/x25519/v1")
        .map_err(|e| e.to_string())?;
    let seed: [u8; 32] = mac
        .as_slice()
        .try_into()
        .map_err(|_| "mac length".to_string())?;
    let x = XPub::from(&StaticSecret::from(seed));
    let key = result_key()?;
    let result = json!({
        "v": 1, "op": "keys",
        "matcher_x25519": hex::encode(x.as_bytes()),
        "venue": VENUE_KEY,
        "result_key": key,
    })
    .to_string();
    let sig = sign_host(&sha256(result.as_bytes()))?;
    Ok(output(&result, &sig, &key))
}

/// Echo envelope: same framing as a sealed order, plaintext not encrypted.
fn open_echo(sealed: &[u8]) -> Option<Value> {
    if sealed.len() < HEADER + 4 + TAG || sealed[0] != 1 {
        return None;
    }
    let body = &sealed[HEADER..sealed.len() - TAG];
    let n = u32::from_le_bytes(body[..4].try_into().ok()?) as usize;
    let v: Value = serde_json::from_slice(body.get(4..4 + n)?).ok()?;
    (v["echo"] == 1).then_some(v)
}

fn batch(input: &Value, input_sha256: &str) -> Result<Value, String> {
    let orders = input["orders"].as_array().cloned().unwrap_or_default();
    let (mut ingested, mut rejected) = (Vec::new(), Vec::new());
    let (mut settle, mut batches) = (Vec::new(), Vec::new());
    let mut lots = 0u64;
    let mut forge = String::from("none");
    for o in &orders {
        let id = o["id"].as_str().unwrap_or_default().to_string();
        let sealed = B64
            .decode(o["sealed"].as_str().unwrap_or_default())
            .unwrap_or_default();
        match open_echo(&sealed) {
            Some(ins) if ins["reject"] != true => {
                ingested.push(id);
                for sb in ins["settle"].as_array().into_iter().flatten() {
                    lots += sb.as_array().map_or(0, |a| a.len() as u64);
                    settle.push(sb.clone());
                }
                batches.extend(ins["batches"].as_array().cloned().unwrap_or_default());
                if let Some(f) = ins["forge"].as_str() {
                    forge = f.to_string();
                }
            }
            _ => rejected.push(json!({ "id": id, "reason": "invalid" })),
        }
    }
    let result = json!({
        "v": 1, "op": "batch", "input_sha256": input_sha256,
        "ingested": ingested, "rejected": rejected, "deferred": [],
        "batches": batches, "settle": settle,
        "book": { "orders": orders.len(), "lots": lots },
    })
    .to_string();
    let key = result_key()?;
    let digest = sha256(result.as_bytes());
    Ok(match forge.as_str() {
        "wrong_key" => {
            let forger = SigningKey::from_bytes(&sha256(b"blindbook/echo/forger/v1"));
            output(&result, &forger.sign(&digest).to_bytes(), &key)
        }
        "tamper" => output(&format!("{result} "), &sign_host(&digest)?, &key),
        _ => output(&result, &sign_host(&digest)?, &key),
    })
}
