//! Prints a deterministic envelope test vector shared with the TypeScript SDK.
use blindbook_core::envelope::{seal, MatcherKey};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    let key = MatcherKey::from_seed([0x11; 32]);
    let body = br#"{"v":1,"side":"sell","lots":[]}"#;
    let sealed = seal(key.public_bytes(), body, [0x22; 32], [0x33; 24]).unwrap();
    println!(
        "{{\"seed\":\"{}\",\"matcherPublicKey\":\"{}\",\"ephemeralSecret\":\"{}\",\"nonce\":\"{}\",\"body\":{},\"sealedSha256\":\"{}\",\"sealedLength\":{}}}",
        hex(&[0x11; 32]),
        hex(&key.public_bytes()),
        hex(&[0x22; 32]),
        hex(&[0x33; 24]),
        serde_json::to_string(std::str::from_utf8(body).unwrap()).unwrap(),
        {
            use sha2::Digest;
            hex(&sha2::Sha256::digest(&sealed))
        },
        sealed.len()
    );
}
