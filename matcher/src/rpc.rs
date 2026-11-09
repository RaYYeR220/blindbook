//! NEAR JSON-RPC `call_function` views over the host's HTTPS.

use serde_json::{json, Value};

use crate::host::{HostError, HttpRequest, HttpResponse};
use crate::util::{b64, unb64};

pub const MAINNET_RPC: &str = "https://rpc.mainnet.fastnear.com";
pub const TESTNET_RPC: &str = "https://test.rpc.fastnear.com";

/// The RPC endpoint for a network, or `None` for an unknown one.
pub fn endpoint(network: &str) -> Option<&'static str> {
    match network {
        "mainnet" => Some(MAINNET_RPC),
        "testnet" => Some(TESTNET_RPC),
        _ => None,
    }
}

/// One view call.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub contract: String,
    pub method: &'static str,
    pub args: Value,
}

impl View {
    pub fn new(contract: &str, method: &'static str, args: Value) -> Self {
        Self {
            contract: contract.to_string(),
            method,
            args,
        }
    }

    pub fn request(&self, rpc_url: &str) -> HttpRequest {
        let body = json!({
            "jsonrpc": "2.0",
            "id": "blindbook",
            "method": "query",
            "params": {
                "request_type": "call_function",
                "finality": "optimistic",
                "account_id": self.contract,
                "method_name": self.method,
                "args_base64": b64(self.args.to_string().as_bytes()),
            }
        });
        HttpRequest {
            url: rpc_url.to_string(),
            body: body.to_string().into_bytes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewError {
    /// The request never got an answer: network, timeout, non-2xx status.
    Transport,
    /// The node answered with an error unrelated to the contract.
    Rpc,
    /// The contract panicked. The text is for classification only and is
    /// never copied into the public output.
    Contract(String),
    /// The answer was not the JSON we expected.
    Decode,
}

/// Decode a JSON-RPC answer to a `call_function` query into the view's
/// return value.
pub fn parse(resp: &Result<HttpResponse, HostError>) -> Result<Value, ViewError> {
    let resp = resp.as_ref().map_err(|_| ViewError::Transport)?;
    if !(200..300).contains(&resp.status) {
        return Err(ViewError::Transport);
    }
    let v: Value = serde_json::from_slice(&resp.body).map_err(|_| ViewError::Decode)?;
    if let Some(err) = v.get("error") {
        // nearcore >= 1.37 reports contract panics as a handler error.
        let cause = err.pointer("/cause/name").and_then(Value::as_str).unwrap_or_default();
        if cause == "CONTRACT_EXECUTION_ERROR" {
            let msg = err
                .pointer("/cause/info/vm_error")
                .map(|m| m.to_string())
                .or_else(|| err.get("data").map(|d| d.to_string()))
                .unwrap_or_default();
            return Err(ViewError::Contract(msg));
        }
        return Err(ViewError::Rpc);
    }
    let result = v.get("result").ok_or(ViewError::Decode)?;
    if let Some(msg) = result.get("error").and_then(Value::as_str) {
        return Err(ViewError::Contract(msg.to_string()));
    }
    let bytes: Vec<u8> = result
        .get("result")
        .and_then(Value::as_array)
        .ok_or(ViewError::Decode)?
        .iter()
        .map(|b| b.as_u64().filter(|b| *b <= 255).map(|b| b as u8))
        .collect::<Option<_>>()
        .ok_or(ViewError::Decode)?;
    serde_json::from_slice(&bytes).map_err(|_| ViewError::Decode)
}

/// Decode a view that returns a JSON boolean.
pub fn parse_bool(resp: &Result<HttpResponse, HostError>) -> Result<bool, ViewError> {
    parse(resp)?.as_bool().ok_or(ViewError::Decode)
}

/// Decode a view that returns a decimal `u128` string.
pub fn parse_u128(resp: &Result<HttpResponse, HostError>) -> Result<u128, ViewError> {
    parse(resp)?
        .as_str()
        .and_then(|s| s.parse::<u128>().ok())
        .ok_or(ViewError::Decode)
}

/// The JSON args of a request built by [`View::request`], for test doubles.
pub fn decode_request(req: &HttpRequest) -> Option<(String, String, Value)> {
    let v: Value = serde_json::from_slice(&req.body).ok()?;
    let p = v.get("params")?;
    let args = unb64(p.get("args_base64")?.as_str()?)?;
    Some((
        p.get("account_id")?.as_str()?.to_string(),
        p.get("method_name")?.as_str()?.to_string(),
        serde_json::from_slice(&args).ok()?,
    ))
}

/// Wrap a view's return value the way a node does, for test doubles.
pub fn encode_result(value: &Value) -> Vec<u8> {
    let bytes: Vec<Value> = value.to_string().into_bytes().into_iter().map(Value::from).collect();
    json!({ "jsonrpc": "2.0", "id": "blindbook", "result": { "result": bytes, "logs": [], "block_height": 1, "block_hash": "11111111111111111111111111111111" } })
        .to_string()
        .into_bytes()
}

/// A contract panic the way a node reports it, for test doubles.
pub fn encode_panic(msg: &str) -> Vec<u8> {
    json!({
        "jsonrpc": "2.0",
        "id": "blindbook",
        "error": {
            "name": "HANDLER_ERROR",
            "cause": { "name": "CONTRACT_EXECUTION_ERROR", "info": { "vm_error": msg } },
            "code": -32000,
            "message": "Server error",
        }
    })
    .to_string()
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(body: Vec<u8>) -> Result<HttpResponse, HostError> {
        Ok(HttpResponse { status: 200, body })
    }

    #[test]
    fn request_carries_the_view_and_round_trips() {
        let view = View::new(
            "intents.near",
            "is_nonce_used",
            json!({ "account_id": "a.near", "nonce": "AAAA" }),
        );
        let req = view.request(MAINNET_RPC);
        assert_eq!(req.url, MAINNET_RPC);
        let (contract, method, args) = decode_request(&req).unwrap();
        assert_eq!(contract, "intents.near");
        assert_eq!(method, "is_nonce_used");
        assert_eq!(args["nonce"], "AAAA");
    }

    #[test]
    fn parses_values_and_errors() {
        assert_eq!(parse_bool(&ok(encode_result(&json!(true)))), Ok(true));
        assert_eq!(
            parse_u128(&ok(encode_result(&json!("123456789012345678901234567")))),
            Ok(123_456_789_012_345_678_901_234_567)
        );
        assert_eq!(parse(&ok(encode_result(&json!("0d9327b2")))), Ok(json!("0d9327b2")));
        assert!(
            matches!(parse(&ok(encode_panic("nonce was already used"))), Err(ViewError::Contract(m)) if m.contains("nonce"))
        );
        // pre-1.37 nodes put the panic inside `result`
        let legacy = json!({ "jsonrpc": "2.0", "id": 1, "result": { "error": "wasm execution failed", "logs": [] } })
            .to_string();
        assert!(matches!(parse(&ok(legacy.into_bytes())), Err(ViewError::Contract(_))));
        let rpc_err = json!({ "jsonrpc": "2.0", "id": 1, "error": { "name": "REQUEST_VALIDATION_ERROR", "cause": { "name": "PARSE_ERROR" } } });
        assert_eq!(parse(&ok(rpc_err.to_string().into_bytes())), Err(ViewError::Rpc));
        assert_eq!(
            parse(&Ok(HttpResponse {
                status: 429,
                body: vec![]
            })),
            Err(ViewError::Transport)
        );
        assert_eq!(parse(&Err(HostError::new("timeout"))), Err(ViewError::Transport));
        assert_eq!(parse(&ok(b"not json".to_vec())), Err(ViewError::Decode));
        assert_eq!(parse_bool(&ok(encode_result(&json!("yes")))), Err(ViewError::Decode));
    }

    #[test]
    fn endpoints() {
        assert_eq!(endpoint("mainnet"), Some(MAINNET_RPC));
        assert_eq!(endpoint("testnet"), Some(TESTNET_RPC));
        assert_eq!(endpoint("betanet"), None);
    }
}
