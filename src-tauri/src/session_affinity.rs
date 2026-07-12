//! Session → account sticky bindings for multi-turn prompt-cache stability.
//!
//! In-memory only (process lifetime). Failures invalidate the binding so the
//! next turn can rebalance without user action.

use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use axum::http::HeaderMap;

static BINDINGS: Lazy<RwLock<HashMap<String, AffinityEntry>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Clone)]
struct AffinityEntry {
    account_id: String,
    expires_at: DateTime<Utc>,
}

/// Extract a **stable** session key for sticky account + prompt cache.
///
/// Priority (first non-empty wins):
/// 1. Client `prompt_cache_key` (best — Codex usually keeps this stable per thread)
/// 2. Conversation / metadata / session headers (stable across turns)
/// 3. Hash of model + first user text (stable for the thread's opening)
/// 4. `previous_response_id` last — only for account sticky chain, not as cache key
///    (it changes every turn; using it as `prompt_cache_key` destroys prefix cache)
pub fn extract_session_key(headers: &HeaderMap, body: Option<&Value>) -> Option<String> {
    if let Some(v) = body {
        if let Some(k) = json_str(v, &["prompt_cache_key"]) {
            return Some(normalize_key(&k));
        }
        if let Some(k) = json_str(v, &["conversation_id"]) {
            return Some(format!("conv:{}", normalize_key(&k)));
        }
        if let Some(meta) = v.get("metadata") {
            if let Some(k) = json_str(meta, &["user_id", "session_id", "conversation_id"]) {
                return Some(format!("meta:{}", normalize_key(&k)));
            }
        }
    }

    for name in [
        "x-session-id",
        "session_id",
        "session-id",
        "x-conversation-id",
        "conversation_id",
    ] {
        if let Some(v) = headers.get(name).and_then(|h| h.to_str().ok()) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(format!("hdr:{}", normalize_key(t)));
            }
        }
    }

    // Stable content seed before previous_response_id (which rotates every turn).
    if let Some(v) = body {
        if let Some(seed) = weak_content_seed(v) {
            return Some(format!("seed:{}", seed));
        }
        if let Some(k) = json_str(v, &["previous_response_id"]) {
            return Some(format!("prev:{}", normalize_key(&k)));
        }
    }
    None
}

/// Value suitable for upstream `prompt_cache_key` / `x-grok-conv-id`.
/// Returns None when the session key is a rotating `previous_response_id` chain key.
pub fn stable_cache_key(session_key: &str) -> Option<String> {
    let t = session_key.trim();
    if t.is_empty() || t.starts_with("prev:") {
        return None;
    }
    let key = t
        .strip_prefix("conv:")
        .or_else(|| t.strip_prefix("meta:"))
        .or_else(|| t.strip_prefix("hdr:"))
        .or_else(|| t.strip_prefix("seed:"))
        .unwrap_or(t);
    let clipped: String = key.chars().take(128).collect();
    if clipped.is_empty() {
        None
    } else {
        Some(clipped)
    }
}

fn weak_content_seed(body: &Value) -> Option<String> {
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
    let first_user = first_user_snippet(body).unwrap_or_default();
    if model.is_empty() && first_user.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    model.hash(&mut hasher);
    first_user.hash(&mut hasher);
    Some(format!("{:x}", hasher.finish()))
}

fn first_user_snippet(body: &Value) -> Option<String> {
    if let Some(arr) = body.get("input").and_then(|v| v.as_array()) {
        for item in arr {
            let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "user" {
                return Some(flatten_text(item.get("content")).chars().take(200).collect());
            }
        }
    }
    if let Some(arr) = body.get("messages").and_then(|v| v.as_array()) {
        for item in arr {
            let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("");
            if role == "user" {
                return Some(flatten_text(item.get("content")).chars().take(200).collect());
            }
        }
    }
    None
}

fn flatten_text(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                    out.push_str(t);
                } else if let Some(s) = p.as_str() {
                    out.push_str(s);
                }
            }
            out
        }
        _ => content.to_string(),
    }
}

fn json_str(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn normalize_key(raw: &str) -> String {
    let t = raw.trim();
    if t.len() > 200 {
        format!("{}…{}", &t[..80], {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            t.hash(&mut hasher);
            format!("{:x}", hasher.finish())
        })
    } else {
        t.to_string()
    }
}

pub fn lookup(session_key: &str) -> Option<String> {
    let now = Utc::now();
    let mut map = BINDINGS.write();
    if map.len() > 4096 {
        map.retain(|_, e| e.expires_at > now);
    }
    let entry = map.get(session_key)?;
    if entry.expires_at <= now {
        map.remove(session_key);
        return None;
    }
    Some(entry.account_id.clone())
}

pub fn bind(session_key: &str, account_id: &str, ttl_secs: u64) {
    if session_key.is_empty() || account_id.is_empty() {
        return;
    }
    let ttl = ttl_secs.clamp(60, 86_400);
    let mut map = BINDINGS.write();
    map.insert(
        session_key.to_string(),
        AffinityEntry {
            account_id: account_id.to_string(),
            expires_at: Utc::now() + Duration::seconds(ttl as i64),
        },
    );
}

/// After a successful response, chain sticky from `previous_response_id` → account
/// and also bind the new response id so the next turn sticks.
pub fn bind_response_chain(response_id: &str, account_id: &str, ttl_secs: u64) {
    let id = response_id.trim();
    if id.is_empty() {
        return;
    }
    bind(&format!("prev:{}", normalize_key(id)), account_id, ttl_secs);
}

pub fn invalidate(session_key: &str) {
    BINDINGS.write().remove(session_key);
}

pub fn invalidate_account(account_id: &str) {
    BINDINGS
        .write()
        .retain(|_, e| e.account_id != account_id);
}

/// Ensure `prompt_cache_key` is present for Responses-style bodies so xAI can
/// reuse prefix cache within a sticky session. Does not overwrite a non-empty client key.
/// Skips rotating `prev:*` keys (would break prefix cache).
pub fn ensure_prompt_cache_key(body: &mut Value, session_key: &str) -> bool {
    let Some(key) = stable_cache_key(session_key) else {
        return false;
    };
    let Some(obj) = body.as_object_mut() else {
        return false;
    };
    let existing = obj
        .get("prompt_cache_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if existing.is_some() {
        return false;
    }
    obj.insert("prompt_cache_key".into(), Value::String(key));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    #[test]
    fn extracts_prompt_cache_key_first() {
        let body = json!({
            "prompt_cache_key": "abc",
            "previous_response_id": "resp_1",
        });
        let headers = HeaderMap::new();
        assert_eq!(
            extract_session_key(&headers, Some(&body)).as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn prefers_seed_over_previous_response_id() {
        let body = json!({
            "model": "grok-4.5",
            "previous_response_id": "resp_rotating",
            "input": [{"role": "user", "content": "hello world for seed"}]
        });
        let headers = HeaderMap::new();
        let key = extract_session_key(&headers, Some(&body)).unwrap();
        assert!(key.starts_with("seed:"), "got {key}");
        assert!(stable_cache_key(&key).is_some());
    }

    #[test]
    fn prev_key_not_used_as_cache_key() {
        assert!(stable_cache_key("prev:resp_xyz").is_none());
        assert_eq!(
            stable_cache_key("seed:deadbeef").as_deref(),
            Some("deadbeef")
        );
    }

    #[test]
    fn bind_lookup_invalidate() {
        bind("k1", "acc-a", 3600);
        assert_eq!(lookup("k1").as_deref(), Some("acc-a"));
        invalidate("k1");
        assert!(lookup("k1").is_none());
    }

    #[test]
    fn ensure_prompt_cache_key_fills_missing() {
        let mut body = json!({"model": "grok-4.5", "input": []});
        assert!(ensure_prompt_cache_key(&mut body, "seed:sess-xyz"));
        assert_eq!(
            body.get("prompt_cache_key").and_then(|v| v.as_str()),
            Some("sess-xyz")
        );
        assert!(!ensure_prompt_cache_key(&mut body, "other"));
    }

    #[test]
    fn ensure_skips_prev_keys() {
        let mut body = json!({"model": "grok-4.5", "input": []});
        assert!(!ensure_prompt_cache_key(&mut body, "prev:resp_1"));
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn header_fallback() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-id", HeaderValue::from_static("s-99"));
        assert_eq!(
            extract_session_key(&headers, None).as_deref(),
            Some("hdr:s-99")
        );
    }
}
