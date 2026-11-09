//! Writes the OutLayer manifest the module embeds in its `outlayer.manifest`
//! custom section. The manifest is covered by the wasm's sha256, so the pool
//! account it admits is fixed per build:
//!
//! - default: testnet, `pool.blindbook-dev.testnet`
//! - feature `mainnet`: `pool.blindbook.near`
//! - env `BLINDBOOK_POOL=<account>`: overrides the pool account
//! - feature `scratch` (testnet only): no `callers` gate, for direct test runs

use std::env;
use std::fs;
use std::path::Path;

const MAINNET_POOL: &str = "pool.blindbook.near";
const TESTNET_POOL: &str = "pool.blindbook-dev.testnet";

/// Outbound hosts: 1Click for reference quotes, FastNEAR for verifier views.
const NETWORK: &[&str] = &[
    "1click.chaindefuser.com",
    "rpc.mainnet.fastnear.com",
    "test.rpc.fastnear.com",
];

fn valid_account(id: &str) -> bool {
    (2..=64).contains(&id.len())
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
                && !part.starts_with(['-', '_'])
                && !part.ends_with(['-', '_'])
        })
}

fn main() {
    println!("cargo:rerun-if-env-changed=BLINDBOOK_POOL");
    println!("cargo:rerun-if-changed=build.rs");

    let mainnet = env::var_os("CARGO_FEATURE_MAINNET").is_some();
    let scratch = env::var_os("CARGO_FEATURE_SCRATCH").is_some();
    if mainnet && scratch {
        panic!("the `scratch` build is testnet-only and cannot be combined with `mainnet`");
    }

    let pool = env::var("BLINDBOOK_POOL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| (if mainnet { MAINNET_POOL } else { TESTNET_POOL }).to_string());
    if !valid_account(&pool) {
        panic!("BLINDBOOK_POOL is not a valid NEAR account id");
    }
    if mainnet != pool.ends_with(".near") {
        panic!(
            "pool account {pool} does not belong to the {} build",
            if mainnet { "mainnet" } else { "testnet" }
        );
    }

    let network: Vec<String> = NETWORK.iter().map(|h| format!("\"{h}\"")).collect();
    let callers = if scratch {
        String::new()
    } else {
        format!(r#","callers":{{"contract":{{"only":["{pool}"]}}}}"#)
    };
    let manifest = format!(
        concat!(
            r#"{{"signing_keys":["#,
            r#"{{"path":"venue","type":"ed25519","caller":"predecessor"}},"#,
            r#"{{"path":"result","type":"ed25519","caller":"predecessor"}}],"#,
            r#""encryption_keys":[{{"path":"book","caller":"predecessor"}}],"#,
            r#""storage_account":"predecessor"{callers},"#,
            r#""capabilities":{{"network":[{network}]}}}}"#
        ),
        callers = callers,
        network = network.join(","),
    );

    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("manifest.json");
    fs::write(&out, manifest).expect("write manifest");
    println!("cargo:rustc-env=BLINDBOOK_POOL_ACCOUNT={pool}");
}
