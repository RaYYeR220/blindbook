//! The real host: OutLayer's storage and keys, WASI HTTP, clock and RNG.

use std::time::{SystemTime, UNIX_EPOCH};

use blindbook_matcher::util::{ed25519_bytes, unb64};
use blindbook_matcher::{Host, HostError, HttpRequest, HttpResponse, KeyId, Nep413Signed, X25519_SEED_CONTEXT};
use outlayer::{encryption_keys, signing_keys, storage};
use wasi::http::outgoing_handler;
use wasi::http::types::{
    ErrorCode, Fields, FutureIncomingResponse, IncomingBody, IncomingResponse, Method, OutgoingBody, OutgoingRequest,
    RequestOptions, Scheme,
};
use wasi::io::poll;
use wasi::io::streams::{OutputStream, StreamError};

/// Requests in flight at once.
const CONCURRENCY: usize = 16;
const MS: u64 = 1_000_000;
const CONNECT_TIMEOUT_NS: u64 = 10_000 * MS;
const FIRST_BYTE_TIMEOUT_NS: u64 = 25_000 * MS;
const BETWEEN_BYTES_TIMEOUT_NS: u64 = 10_000 * MS;
/// A group of concurrent requests that has not completed by then is abandoned.
const GROUP_TIMEOUT_NS: u64 = 40_000 * MS;
/// Largest response body read.
const MAX_BODY: usize = 4 << 20;

fn err(msg: impl Into<String>) -> HostError {
    HostError::new(msg)
}

pub struct OutLayerHost;

impl Host for OutLayerHost {
    fn storage_get(&mut self, key: &str) -> Result<Option<Vec<u8>>, HostError> {
        storage::get(key).map_err(|e| err(e.to_string()))
    }

    fn storage_put(&mut self, key: &str, expected: Option<&[u8]>, value: &[u8]) -> Result<bool, HostError> {
        match expected {
            None => storage::set_if_absent(key, value),
            Some(prev) => storage::set_if_equals(key, prev, value).map(|(ok, _)| ok),
        }
        .map_err(|e| err(e.to_string()))
    }

    fn x25519_seed(&mut self) -> Result<[u8; 32], HostError> {
        let mac = encryption_keys::mac("book", None, X25519_SEED_CONTEXT).map_err(|e| err(e.to_string()))?;
        mac.try_into().map_err(|_| err("mac length"))
    }

    fn public_key(&mut self, key: KeyId) -> Result<[u8; 32], HostError> {
        let pk = signing_keys::public_key(key.path(), None).map_err(|e| err(e.to_string()))?;
        pk.try_into().map_err(|_| err("public key length"))
    }

    fn sign_result(&mut self, digest: &[u8; 32]) -> Result<[u8; 64], HostError> {
        let sig = signing_keys::sign(KeyId::Result.path(), None, digest).map_err(|e| err(e.to_string()))?;
        sig.try_into().map_err(|_| err("signature length"))
    }

    fn sign_venue_nep413(
        &mut self,
        message: &str,
        recipient: &str,
        nonce: &[u8; 32],
    ) -> Result<Nep413Signed, HostError> {
        let s = signing_keys::sign_nep413(KeyId::Venue.path(), None, message, recipient, nonce, None)
            .map_err(|e| err(e.to_string()))?;
        let public_key = ed25519_bytes(&s.public_key)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| err("nep413 public key"))?;
        let signature = unb64(&s.signature)
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| err("nep413 signature"))?;
        Ok(Nep413Signed { public_key, signature })
    }

    fn http_post(&mut self, requests: &[HttpRequest]) -> Vec<Result<HttpResponse, HostError>> {
        requests.chunks(CONCURRENCY).flat_map(send_group).collect()
    }

    fn now_ns(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos().min(u64::MAX as u128) as u64)
            .unwrap_or(0)
    }

    fn random(&mut self, out: &mut [u8]) -> Result<(), HostError> {
        let bytes = wasi::random::random::get_random_bytes(out.len() as u64);
        if bytes.len() != out.len() {
            return Err(err("short random read"));
        }
        out.copy_from_slice(&bytes);
        Ok(())
    }
}

/// Start every request, then wait for all answers (or the group timeout).
fn send_group(requests: &[HttpRequest]) -> Vec<Result<HttpResponse, HostError>> {
    let futures: Vec<Result<FutureIncomingResponse, HostError>> = requests.iter().map(start).collect();
    let mut out: Vec<Option<Result<HttpResponse, HostError>>> = futures
        .iter()
        .map(|f| f.as_ref().err().map(|e| Err(e.clone())))
        .collect();
    let timer = wasi::clocks::monotonic_clock::subscribe_duration(GROUP_TIMEOUT_NS);
    loop {
        let pending: Vec<usize> = (0..futures.len()).filter(|i| out[*i].is_none()).collect();
        if pending.is_empty() {
            break;
        }
        let mut done_any = false;
        for &i in &pending {
            if let Some(Ok(f)) = futures.get(i) {
                if let Some(r) = f.get() {
                    out[i] = Some(read(r));
                    done_any = true;
                }
            }
        }
        if done_any {
            continue;
        }
        let pollables: Vec<_> = pending
            .iter()
            .filter_map(|i| futures[*i].as_ref().ok().map(FutureIncomingResponse::subscribe))
            .collect();
        let mut refs: Vec<&poll::Pollable> = pollables.iter().collect();
        refs.push(&timer);
        let ready = poll::poll(&refs);
        drop(refs);
        drop(pollables);
        if ready.iter().any(|r| *r as usize == pending.len()) {
            for i in pending {
                if out[i].is_none() {
                    out[i] = Some(Err(err("timeout")));
                }
            }
            break;
        }
    }
    drop(timer);
    out.into_iter()
        .map(|r| r.unwrap_or_else(|| Err(err("no response"))))
        .collect()
}

fn start(req: &HttpRequest) -> Result<FutureIncomingResponse, HostError> {
    let rest = req.url.strip_prefix("https://").ok_or_else(|| err("https only"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let mut entries = vec![
        ("content-type".to_string(), b"application/json".to_vec()),
        ("accept".to_string(), b"application/json".to_vec()),
        ("user-agent".to_string(), b"blindbook-matcher/0.1".to_vec()),
        ("content-length".to_string(), req.body.len().to_string().into_bytes()),
    ];
    let headers = match Fields::from_list(&entries) {
        Ok(h) => h,
        Err(_) => {
            entries.pop();
            Fields::from_list(&entries).map_err(|_| err("headers"))?
        }
    };
    let request = OutgoingRequest::new(headers);
    request.set_method(&Method::Post).map_err(|_| err("method"))?;
    request.set_scheme(Some(&Scheme::Https)).map_err(|_| err("scheme"))?;
    request.set_authority(Some(authority)).map_err(|_| err("authority"))?;
    request.set_path_with_query(Some(path)).map_err(|_| err("path"))?;
    let body = request.body().map_err(|_| err("body"))?;
    let options = RequestOptions::new();
    let _ = options.set_connect_timeout(Some(CONNECT_TIMEOUT_NS));
    let _ = options.set_first_byte_timeout(Some(FIRST_BYTE_TIMEOUT_NS));
    let _ = options.set_between_bytes_timeout(Some(BETWEEN_BYTES_TIMEOUT_NS));
    // Hand the request over first, then stream the body into it: the host
    // drains the body as it sends, so writing never waits on a full buffer.
    let future = outgoing_handler::handle(request, Some(options)).map_err(|c| err(format!("{c:?}")))?;
    {
        let stream = body.write().map_err(|_| err("body stream"))?;
        write_all(&stream, &req.body)?;
    }
    OutgoingBody::finish(body, None).map_err(|c| err(format!("{c:?}")))?;
    Ok(future)
}

fn write_all(stream: &OutputStream, mut bytes: &[u8]) -> Result<(), HostError> {
    while !bytes.is_empty() {
        let permit = stream.check_write().map_err(|e| err(format!("{e:?}")))? as usize;
        if permit == 0 {
            stream.subscribe().block();
            continue;
        }
        let n = permit.min(bytes.len());
        stream.write(&bytes[..n]).map_err(|e| err(format!("{e:?}")))?;
        bytes = &bytes[n..];
    }
    stream.blocking_flush().map_err(|e| err(format!("{e:?}")))
}

fn read(r: Result<Result<IncomingResponse, ErrorCode>, ()>) -> Result<HttpResponse, HostError> {
    let resp = r
        .map_err(|_| err("response taken"))?
        .map_err(|c| err(format!("{c:?}")))?;
    let status = resp.status();
    let body = resp.consume().map_err(|_| err("consume"))?;
    drop(resp);
    let mut buf = Vec::new();
    {
        let stream = body.stream().map_err(|_| err("body stream"))?;
        loop {
            match stream.blocking_read(64 * 1024) {
                Ok(chunk) => {
                    buf.extend_from_slice(&chunk);
                    if buf.len() > MAX_BODY {
                        return Err(err("body too large"));
                    }
                }
                Err(StreamError::Closed) => break,
                Err(e) => return Err(err(format!("{e:?}"))),
            }
        }
    }
    let _ = IncomingBody::finish(body);
    Ok(HttpResponse { status, body: buf })
}
