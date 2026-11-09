//! NEP-297 events: `EVENT_JSON:{"standard":"blindbook","version":"1.0.0","event":...,"data":[...]}`.

use near_sdk::env;
use near_sdk::serde::Serialize;

pub const STANDARD: &str = "blindbook";
pub const VERSION: &str = "1.0.0";

pub fn emit<T: Serialize>(event: &str, data: &T) {
    let data = serde_json::to_string(data).unwrap_or_else(|_| env::abort());
    env::log_str(&format!(
        r#"EVENT_JSON:{{"standard":"{STANDARD}","version":"{VERSION}","event":"{event}","data":[{data}]}}"#
    ));
}
