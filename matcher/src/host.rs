//! Everything the matcher needs from its environment.
//!
//! The real implementation (`main.rs`, `wasm32-wasip2`) maps each method onto
//! the OutLayer host: keystore-encrypted storage in the predecessor's cell,
//! host-held signing and encryption keys, outbound HTTPS, the wall clock and
//! the system RNG. Tests use an in-memory mock. Keeping the trait narrow keeps
//! the keys narrow too: the result key can only sign a 32-byte digest and the
//! venue key can only make NEP-413 signatures.

/// The host signing keys the manifest declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyId {
    /// Signs the venue's NEP-413 payload in every sub-batch.
    Venue,
    /// Signs `sha256(result)`, which the pool checks before acting.
    Result,
}

impl KeyId {
    /// The manifest path of the key.
    pub fn path(self) -> &'static str {
        match self {
            KeyId::Venue => "venue",
            KeyId::Result => "result",
        }
    }
}

/// A host failure. Its text is for debugging only and never reaches the
/// public output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError(pub String);

impl HostError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// An HTTPS `POST` with a JSON body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// A NEP-413 signature made by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nep413Signed {
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}

pub trait Host {
    /// Read a record of the run's storage cell. `Ok(None)` when absent.
    fn storage_get(&mut self, key: &str) -> Result<Option<Vec<u8>>, HostError>;

    /// Compare-and-swap write: store `value` only if the record still holds
    /// `expected` (`None`: only if it is still absent). `Ok(false)` when another
    /// run changed it in between.
    fn storage_put(&mut self, key: &str, expected: Option<&[u8]>, value: &[u8]) -> Result<bool, HostError>;

    /// The 32-byte seed of the matcher's X25519 order key:
    /// `encryption_keys::mac("book", None, "blindbook/v1/matcher-x25519")`.
    fn x25519_seed(&mut self) -> Result<[u8; 32], HostError>;

    /// The public key of a host signing key.
    fn public_key(&mut self, key: KeyId) -> Result<[u8; 32], HostError>;

    /// ed25519 signature by the `result` key over the 32 bytes of `digest`.
    fn sign_result(&mut self, digest: &[u8; 32]) -> Result<[u8; 64], HostError>;

    /// NEP-413 signature by the `venue` key over
    /// `{message, nonce, recipient, callback_url: None}`.
    fn sign_venue_nep413(
        &mut self,
        message: &str,
        recipient: &str,
        nonce: &[u8; 32],
    ) -> Result<Nep413Signed, HostError>;

    /// Send every request, concurrently where the host can, and answer in the
    /// same order. One failure never fails the others.
    fn http_post(&mut self, requests: &[HttpRequest]) -> Vec<Result<HttpResponse, HostError>>;

    /// Wall-clock time in Unix nanoseconds.
    fn now_ns(&mut self) -> u64;

    /// Fill `out` with cryptographically secure random bytes.
    fn random(&mut self, out: &mut [u8]) -> Result<(), HostError>;
}
