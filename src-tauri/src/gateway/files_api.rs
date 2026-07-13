//! xAI Files API client + in-memory content-hash → file_id cache.
//!
//! Research (xAI docs + CPA/sub2api patterns):
//! - Documents should be uploaded once via `POST /v1/files` and referenced with
//!   `{ "type": "input_file", "file_id": "..." }` instead of re-inlining bytes
//!   into every multi-turn Responses payload.
//! - Server-side `attachment_search` then processes content; tokens stay lower
//!   and multi-turn agent loops stop exploding.
//! - Images for vision still use `input_image` (base64/URL); CPA notes that
//!   `file_id` is **not** accepted on xAI image/video generation endpoints.
//! - Files are account-scoped → cache keys include account id and we keep
//!   session sticky so later turns hit the same credential.

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use reqwest::multipart::{Form, Part};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult};

/// Default TTL when offloading agent tool output (1 day).
pub const DEFAULT_OFFLOAD_TTL_SECS: u64 = 86_400;

/// Cache entries live a bit shorter than upload TTL so we don't reuse expired ids.
const CACHE_TTL: Duration = Duration::from_secs(80_000);

static FILE_CACHE: Lazy<RwLock<HashMap<String, CacheEntry>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

#[derive(Clone)]
struct CacheEntry {
    file_id: String,
    expires_at: Instant,
}

fn cache_key(account_id: &str, content_hash: &str) -> String {
    format!("{account_id}:{content_hash}")
}

pub fn content_sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn cache_lookup(account_id: &str, content_hash: &str) -> Option<String> {
    let key = cache_key(account_id, content_hash);
    let mut map = FILE_CACHE.write();
    if let Some(entry) = map.get(&key) {
        if entry.expires_at > Instant::now() {
            return Some(entry.file_id.clone());
        }
        map.remove(&key);
    }
    None
}

pub fn cache_insert(account_id: &str, content_hash: &str, file_id: &str) {
    let key = cache_key(account_id, content_hash);
    FILE_CACHE.write().insert(
        key,
        CacheEntry {
            file_id: file_id.to_string(),
            expires_at: Instant::now() + CACHE_TTL,
        },
    );
}

/// Upload raw bytes to xAI Files API. Returns file id.
pub async fn upload_file(
    client: &reqwest::Client,
    xai_base_url: &str,
    token: &str,
    filename: &str,
    bytes: Vec<u8>,
    purpose: Option<&str>,
    expires_after_secs: Option<u64>,
) -> AppResult<String> {
    let url = format!(
        "{}/files",
        xai_base_url.trim_end_matches('/')
    );
    // xAI requires text fields (especially expires_after) BEFORE the file part:
    // {"error":"expires_after must appear before the file field in the multipart form"}
    let mut form = Form::new();
    if let Some(p) = purpose.filter(|s| !s.is_empty()) {
        form = form.text("purpose", p.to_string());
    } else {
        form = form.text("purpose", "assistants".to_string());
    }
    if let Some(ttl) = expires_after_secs {
        // Clamp to xAI allowed range [3600, 2592000].
        let ttl = ttl.clamp(3600, 2_592_000);
        form = form.text("expires_after", ttl.to_string());
    }
    let part = Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str("application/octet-stream")
        .map_err(|e| AppError::msg(format!("multipart part: {e}")))?;
    form = form.part("file", part);

    let resp = client
        .post(&url)
        .bearer_auth(token)
        .multipart(form)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| AppError::msg(format!("files upload request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .unwrap_or_else(|_| String::new());
    if !status.is_success() {
        return Err(AppError::msg(format!(
            "files upload HTTP {}: {}",
            status.as_u16(),
            body.chars().take(400).collect::<String>()
        )));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|e| AppError::msg(format!("files upload JSON parse: {e}; body={body}")))?;
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::msg(format!("files upload missing id: {body}")))?
        .to_string();
    Ok(id)
}

/// Upload with content-hash cache scoped to account.
pub async fn upload_cached(
    client: &reqwest::Client,
    xai_base_url: &str,
    token: &str,
    account_id: &str,
    filename: &str,
    bytes: Vec<u8>,
    expires_after_secs: Option<u64>,
) -> AppResult<String> {
    let hash = content_sha256_hex(&bytes);
    if let Some(id) = cache_lookup(account_id, &hash) {
        tracing::debug!(account = %account_id, file_id = %id, "files cache hit");
        return Ok(id);
    }
    let id = upload_file(
        client,
        xai_base_url,
        token,
        filename,
        bytes,
        Some("assistants"),
        expires_after_secs.or(Some(DEFAULT_OFFLOAD_TTL_SECS)),
    )
    .await?;
    cache_insert(account_id, &hash, &id);
    tracing::info!(
        account = %account_id,
        file_id = %id,
        hash = %hash,
        "uploaded large blob to xAI Files API"
    );
    Ok(id)
}

/// Proxy helpers for gateway routes — list/get/delete pass through JSON.
pub async fn proxy_files_json(
    client: &reqwest::Client,
    xai_base_url: &str,
    token: &str,
    method: reqwest::Method,
    path_and_query: &str,
    body: Option<Value>,
) -> AppResult<(u16, Value)> {
    let url = format!(
        "{}{}",
        xai_base_url.trim_end_matches('/'),
        if path_and_query.starts_with('/') {
            path_and_query.to_string()
        } else {
            format!("/{path_and_query}")
        }
    );
    let mut req = client
        .request(method, &url)
        .bearer_auth(token)
        .timeout(Duration::from_secs(60));
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::msg(format!("files proxy failed: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}))
    };
    Ok((status, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip() {
        let acc = "acc-test-files";
        let hash = content_sha256_hex(b"hello");
        assert!(cache_lookup(acc, &hash).is_none());
        cache_insert(acc, &hash, "file_abc");
        assert_eq!(cache_lookup(acc, &hash).as_deref(), Some("file_abc"));
    }
}
