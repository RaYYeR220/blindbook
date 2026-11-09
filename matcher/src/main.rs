//! Entry point inside OutLayer (`wasm32-wasip2`): stdin JSON in, one JSON
//! object out. Natively it only explains itself; the logic is tested through
//! the library with a mock host.

#[cfg(target_os = "wasi")]
mod wasi_host;

/// The manifest, embedded in a custom section covered by the wasm's sha256.
/// `#[used]` keeps the linker from dropping a static nothing references.
#[cfg(target_os = "wasi")]
#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!(concat!(env!("OUT_DIR"), "/manifest.json")).len()] =
    *include_bytes!(concat!(env!("OUT_DIR"), "/manifest.json"));

#[cfg(target_os = "wasi")]
fn main() {
    use std::io::Write;
    // stderr is public on chain: a panic must not print its message, which
    // could carry state. The run then fails and the pool sees no result.
    std::panic::set_hook(Box::new(|_| {
        let _ = std::io::stderr().write_all(b"matcher: internal error\n");
    }));
    let input = outlayer::env::input();
    let mut host = wasi_host::OutLayerHost;
    let out = blindbook_matcher::run(&mut host, &input);
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!(
        "blindbook-matcher runs inside OutLayer: build it with `cargo build -p blindbook-matcher --target wasm32-wasip2 --release`"
    );
    std::process::exit(2);
}
