//! SuperGrok / Grok Build weekly credit quota.
//!
//! Source of truth for the grok.com "Weekly SuperGrok Limit" UI:
//! `POST https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig`
//! (empty gRPC-web request, Bearer OAuth access token).
//!
//! This is independent of `api.x.ai` `x-ratelimit-*` RPM/TPM headers.

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::auth::ensure_fresh_token;
use crate::config::{load_auth, load_config, save_auth, Account};
use crate::error::{AppError, AppResult};
use crate::http_client::build_http_client;

const BILLING_URL: &str = "https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig";
/// Empty protobuf message framed as gRPC-web (flags=0, length=0).
const EMPTY_GRPC_WEB_FRAME: &[u8] = &[0x00, 0x00, 0x00, 0x00, 0x00];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaProductUsage {
    /// Upstream product id: 1 ≈ API, 2 ≈ Grok Build (heuristic from grok.com UI).
    pub product_id: u32,
    /// Display label when known.
    pub label: String,
    /// Share of the weekly SuperGrok limit attributed to this product (0–100).
    pub used_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountQuotaSnapshot {
    /// Total SuperGrok weekly limit already used (0–100+).
    pub used_percent: f32,
    /// Remaining percent of weekly limit (max(0, 100 - used)).
    pub remaining_percent: f32,
    pub period_start_at: Option<DateTime<Utc>>,
    pub resets_at: Option<DateTime<Utc>>,
    pub products: Vec<QuotaProductUsage>,
    pub fetched_at: DateTime<Utc>,
    /// Optional short error from last failed refresh (cleared on success).
    #[serde(default)]
    pub last_error: Option<String>,
}

impl AccountQuotaSnapshot {
    pub fn from_used(used_percent: f32, period_start_at: Option<DateTime<Utc>>, resets_at: Option<DateTime<Utc>>, products: Vec<QuotaProductUsage>) -> Self {
        let used = sanitize_percent(used_percent);
        Self {
            used_percent: used,
            remaining_percent: (100.0 - used).max(0.0),
            period_start_at,
            resets_at,
            products,
            fetched_at: Utc::now(),
            last_error: None,
        }
    }
}

fn product_label(id: u32) -> String {
    match id {
        1 => "API".into(),
        2 => "Grok Build".into(),
        4 => "Other".into(),
        _ => format!("Product {id}"),
    }
}

fn sanitize_percent(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    // Allow slight overflow past 100 if upstream reports it; clamp display math elsewhere.
    v.clamp(0.0, 200.0)
}

/// Refresh SuperGrok quota for one account and persist onto auth.json.
pub async fn refresh_account_quota(account_id: &str) -> AppResult<Account> {
    let config = load_config()?;
    let store = load_auth()?;
    let exists = store.accounts.iter().any(|a| a.id == account_id);
    if !exists {
        return Err(AppError::msg("account not found"));
    }
    let signed_in = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .map(|a| a.access_token.is_some() || a.refresh_token.is_some())
        .unwrap_or(false);
    if !signed_in {
        return Err(AppError::msg("account is not signed in"));
    }
    refresh_account_quota_inner(&config, account_id).await?;
    let store = load_auth()?;
    store
        .accounts
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| AppError::msg("account not found"))
}

/// Refresh every signed-in account. Per-account failures are stored on `quota.last_error`.
pub async fn refresh_all_account_quotas() -> AppResult<Vec<Account>> {
    let config = load_config()?;
    let store = load_auth()?;
    let ids: Vec<String> = store
        .accounts
        .iter()
        .filter(|a| a.access_token.is_some() || a.refresh_token.is_some())
        .map(|a| a.id.clone())
        .collect();

    for id in ids {
        // Ignore individual errors; each failure still writes last_error onto the account.
        let _ = refresh_account_quota_inner(&config, &id).await;
    }
    Ok(load_auth()?.accounts)
}

async fn refresh_account_quota_inner(config: &crate::config::AppConfig, account_id: &str) -> AppResult<()> {
    let mut store = load_auth()?;
    let account = store
        .accounts
        .iter_mut()
        .find(|a| a.id == account_id)
        .ok_or_else(|| AppError::msg("account not found"))?;

    let token = match ensure_fresh_token(config, account).await {
        Ok(t) => t,
        Err(err) => {
            stamp_quota_error(account, err.to_string());
            save_auth(&store)?;
            return Err(err);
        }
    };

    match fetch_quota_snapshot(config, &token).await {
        Ok(snap) => {
            account.quota = Some(snap);
            save_auth(&store)?;
            Ok(())
        }
        Err(err) => {
            stamp_quota_error(account, err.to_string());
            save_auth(&store)?;
            Err(err)
        }
    }
}

fn stamp_quota_error(account: &mut Account, msg: String) {
    let prev = account.quota.clone();
    account.quota = Some(AccountQuotaSnapshot {
        used_percent: prev.as_ref().map(|q| q.used_percent).unwrap_or(0.0),
        remaining_percent: prev.as_ref().map(|q| q.remaining_percent).unwrap_or(0.0),
        period_start_at: prev.as_ref().and_then(|q| q.period_start_at),
        resets_at: prev.as_ref().and_then(|q| q.resets_at),
        products: prev.as_ref().map(|q| q.products.clone()).unwrap_or_default(),
        fetched_at: Utc::now(),
        last_error: Some(msg),
    });
}

pub async fn fetch_quota_snapshot(
    config: &crate::config::AppConfig,
    access_token: &str,
) -> AppResult<AccountQuotaSnapshot> {
    let client = build_http_client(config)?;
    let resp = client
        .post(BILLING_URL)
        .timeout(Duration::from_secs(25))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .header("x-user-agent", "connect-es/2.1.1")
        .header("Origin", "https://grok.com")
        .header("Referer", "https://grok.com/?_s=usage")
        .header("Accept", "*/*")
        .body(EMPTY_GRPC_WEB_FRAME.to_vec())
        .send()
        .await
        .map_err(|e| AppError::msg(format!("quota request failed: {e}")))?;

    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("quota response body: {e}")))?;

    if let Some(grpc_status) = header_str(&headers, "grpc-status") {
        if grpc_status != "0" {
            let msg = header_str(&headers, "grpc-message").unwrap_or_default();
            return Err(AppError::msg(format!(
                "quota RPC status {grpc_status}: {msg}"
            )));
        }
    }

    if !status.is_success() {
        let preview = String::from_utf8_lossy(&bytes);
        return Err(AppError::msg(format!(
            "quota HTTP {}: {}",
            status.as_u16(),
            truncate(&preview, 200)
        )));
    }

    validate_grpc_web_trailers(&bytes)?;
    parse_grpc_web_quota(&bytes)
}

fn header_str(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            percent_decode_lightweight(s.trim())
        })
}

fn percent_decode_lightweight(s: &str) -> String {
    // grpc-message is often percent-encoded (e.g. Missing%20request).
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

fn validate_grpc_web_trailers(data: &[u8]) -> AppResult<()> {
    let trailers = grpc_web_trailer_fields(data);
    if let Some(status) = trailers.get("grpc-status") {
        if status != "0" {
            let msg = trailers.get("grpc-message").cloned().unwrap_or_default();
            return Err(AppError::msg(format!("quota RPC trailer status {status}: {msg}")));
        }
    }
    Ok(())
}

fn grpc_web_trailer_fields(data: &[u8]) -> std::collections::BTreeMap<String, String> {
    let mut fields = std::collections::BTreeMap::new();
    let mut index = 0usize;
    while index + 5 <= data.len() {
        let flags = data[index];
        let length = u32::from_be_bytes([
            data[index + 1],
            data[index + 2],
            data[index + 3],
            data[index + 4],
        ]) as usize;
        let start = index + 5;
        let end = start.saturating_add(length);
        if end > data.len() {
            break;
        }
        if flags & 0x80 != 0 {
            if let Ok(text) = std::str::from_utf8(&data[start..end]) {
                for line in text.split(|c| c == '\n' || c == '\r') {
                    if line.is_empty() {
                        continue;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        fields.insert(
                            k.trim().to_ascii_lowercase(),
                            percent_decode_lightweight(v.trim()),
                        );
                    }
                }
            }
        }
        index = end;
    }
    fields
}

fn grpc_web_data_frames(data: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut index = 0usize;
    while index + 5 <= data.len() {
        let flags = data[index];
        let length = u32::from_be_bytes([
            data[index + 1],
            data[index + 2],
            data[index + 3],
            data[index + 4],
        ]) as usize;
        let start = index + 5;
        let end = start.saturating_add(length);
        if end > data.len() {
            return Vec::new();
        }
        if flags & 0x80 == 0 {
            frames.push(data[start..end].to_vec());
        }
        index = end;
    }
    frames
}

fn looks_like_protobuf(data: &[u8]) -> bool {
    let Some(&first) = data.first() else {
        return false;
    };
    let field_number = first >> 3;
    let wire_type = first & 0x07;
    field_number > 0 && matches!(wire_type, 0 | 1 | 2 | 5)
}

/// Parse gRPC-web (or raw protobuf) GetGrokCreditsConfig response.
pub fn parse_grpc_web_quota(data: &[u8]) -> AppResult<AccountQuotaSnapshot> {
    let mut payloads = grpc_web_data_frames(data);
    if payloads.is_empty() && looks_like_protobuf(data) {
        payloads = vec![data.to_vec()];
    }
    if payloads.is_empty() {
        return Err(AppError::msg("quota response empty"));
    }

    let mut used_percent: Option<f32> = None;
    let mut period_start: Option<DateTime<Utc>> = None;
    let mut resets_at: Option<DateTime<Utc>> = None;
    let products: Vec<QuotaProductUsage> = Vec::new();

    for payload in &payloads {
        // Preferred structured parse of field 1 message (observed layout).
        if let Some(parsed) = try_parse_credits_config(payload) {
            return Ok(parsed);
        }
        // Fallback: CodexBar-style field scan.
        let scan = scan_protobuf(payload, 0, &[]);
        if used_percent.is_none() {
            used_percent = scan
                .fixed32
                .iter()
                .filter(|f| f.path.last() == Some(&1) && (0.0..=100.0).contains(&f.value))
                .min_by(|a, b| {
                    a.path
                        .len()
                        .cmp(&b.path.len())
                        .then(a.order.cmp(&b.order))
                })
                .map(|f| f.value);
        }
        let epochs: Vec<(Vec<u64>, DateTime<Utc>)> = scan
            .varints
            .iter()
            .filter_map(|f| {
                let raw = f.value;
                if (1_700_000_000..=2_100_000_000).contains(&raw) {
                    Utc.timestamp_opt(raw as i64, 0).single().map(|dt| (f.path.clone(), dt))
                } else {
                    None
                }
            })
            .collect();
        if resets_at.is_none() {
            let now = Utc::now();
            let future: Vec<_> = epochs.iter().filter(|(_, d)| *d > now).collect();
            resets_at = future
                .iter()
                .find(|(p, _)| p.as_slice() == [1, 5, 1])
                .map(|(_, d)| *d)
                .or_else(|| future.iter().map(|(_, d)| *d).min())
                .or_else(|| epochs.iter().map(|(_, d)| *d).max());
        }
        if period_start.is_none() {
            period_start = epochs
                .iter()
                .find(|(p, _)| p.as_slice() == [1, 4, 1])
                .map(|(_, d)| *d)
                .or_else(|| epochs.iter().map(|(_, d)| *d).min());
        }
    }

    let used = used_percent.ok_or_else(|| AppError::msg("could not parse quota percent"))?;
    Ok(AccountQuotaSnapshot::from_used(used, period_start, resets_at, products))
}

/// Structured parse matching observed GetGrokCreditsConfig payload layout.
fn try_parse_credits_config(payload: &[u8]) -> Option<AccountQuotaSnapshot> {
    // Expect: field 1 (len-delimited) wrapping the config message.
    let mut i = 0usize;
    let (key, next) = read_varint(payload, i)?;
    i = next;
    let field = key >> 3;
    let wire = key & 7;
    if field != 1 || wire != 2 {
        return None;
    }
    let (len, next) = read_varint(payload, i)?;
    i = next;
    let end = i.saturating_add(len as usize);
    if end > payload.len() {
        return None;
    }
    let inner = &payload[i..end];

    let mut used: Option<f32> = None;
    let mut period_start: Option<DateTime<Utc>> = None;
    let mut resets_at: Option<DateTime<Utc>> = None;
    let mut products: Vec<QuotaProductUsage> = Vec::new();

    let mut j = 0usize;
    while j < inner.len() {
        let (k, n) = match read_varint(inner, j) {
            Some(v) => v,
            None => break,
        };
        j = n;
        let fn_ = k >> 3;
        let wt = k & 7;
        match (fn_, wt) {
            (1, 5) => {
                if j + 4 > inner.len() {
                    break;
                }
                used = Some(f32::from_le_bytes(inner[j..j + 4].try_into().ok()?));
                j += 4;
            }
            (4, 2) | (5, 2) => {
                let (ln, n) = read_varint(inner, j)?;
                j = n;
                let e = j.saturating_add(ln as usize);
                if e > inner.len() {
                    break;
                }
                let ts = parse_timestamp_message(&inner[j..e]);
                j = e;
                if fn_ == 4 {
                    period_start = ts;
                } else {
                    resets_at = ts;
                }
            }
            (7, 2) => {
                let (ln, n) = read_varint(inner, j)?;
                j = n;
                let e = j.saturating_add(ln as usize);
                if e > inner.len() {
                    break;
                }
                if let Some(p) = parse_product_message(&inner[j..e]) {
                    products.push(p);
                }
                j = e;
            }
            (_, 0) => {
                let (_, n) = read_varint(inner, j)?;
                j = n;
            }
            (_, 1) => {
                j = j.saturating_add(8);
            }
            (_, 2) => {
                let (ln, n) = read_varint(inner, j)?;
                j = n.saturating_add(ln as usize);
            }
            (_, 5) => {
                j = j.saturating_add(4);
            }
            _ => break,
        }
    }

    let used = used?;
    // Product without percent still listed with 0 for visibility of known id.
    for p in &mut products {
        if !p.used_percent.is_finite() {
            p.used_percent = 0.0;
        }
    }
    Some(AccountQuotaSnapshot::from_used(used, period_start, resets_at, products))
}

fn parse_timestamp_message(msg: &[u8]) -> Option<DateTime<Utc>> {
    let mut i = 0usize;
    let mut seconds: Option<i64> = None;
    let mut nanos: u32 = 0;
    while i < msg.len() {
        let (k, n) = read_varint(msg, i)?;
        i = n;
        let fn_ = k >> 3;
        let wt = k & 7;
        match (fn_, wt) {
            (1, 0) => {
                let (v, n) = read_varint(msg, i)?;
                i = n;
                seconds = Some(v as i64);
            }
            (2, 0) => {
                let (v, n) = read_varint(msg, i)?;
                i = n;
                nanos = v as u32;
            }
            (_, 0) => {
                let (_, n) = read_varint(msg, i)?;
                i = n;
            }
            (_, 2) => {
                let (ln, n) = read_varint(msg, i)?;
                i = n.saturating_add(ln as usize);
            }
            (_, 5) => i = i.saturating_add(4),
            (_, 1) => i = i.saturating_add(8),
            _ => break,
        }
    }
    let secs = seconds?;
    Utc.timestamp_opt(secs, nanos).single()
}

fn parse_product_message(msg: &[u8]) -> Option<QuotaProductUsage> {
    let mut i = 0usize;
    let mut product_id: Option<u32> = None;
    let mut used_percent: Option<f32> = None;
    while i < msg.len() {
        let (k, n) = read_varint(msg, i)?;
        i = n;
        let fn_ = k >> 3;
        let wt = k & 7;
        match (fn_, wt) {
            (1, 0) => {
                let (v, n) = read_varint(msg, i)?;
                i = n;
                product_id = Some(v as u32);
            }
            (2, 5) => {
                if i + 4 > msg.len() {
                    break;
                }
                used_percent = Some(f32::from_le_bytes(msg[i..i + 4].try_into().ok()?));
                i += 4;
            }
            (_, 0) => {
                let (_, n) = read_varint(msg, i)?;
                i = n;
            }
            (_, 2) => {
                let (ln, n) = read_varint(msg, i)?;
                i = n.saturating_add(ln as usize);
            }
            (_, 5) => i = i.saturating_add(4),
            (_, 1) => i = i.saturating_add(8),
            _ => break,
        }
    }
    let id = product_id?;
    Some(QuotaProductUsage {
        product_id: id,
        label: product_label(id),
        used_percent: used_percent.unwrap_or(0.0),
    })
}

#[derive(Default)]
struct ProtoScan {
    fixed32: Vec<Fixed32Field>,
    varints: Vec<VarintField>,
}

struct Fixed32Field {
    path: Vec<u64>,
    value: f32,
    order: usize,
}

struct VarintField {
    path: Vec<u64>,
    value: u64,
}

fn scan_protobuf(data: &[u8], depth: usize, path: &[u64]) -> ProtoScan {
    let mut scan = ProtoScan::default();
    let mut index = 0usize;
    let mut order = 0usize;
    while index < data.len() {
        let start = index;
        let Some((key, next)) = read_varint(data, index) else {
            break;
        };
        index = next;
        if key == 0 {
            index = start + 1;
            continue;
        }
        let field_number = key >> 3;
        let wire_type = key & 7;
        let field_path = {
            let mut p = path.to_vec();
            p.push(field_number);
            p
        };
        match wire_type {
            0 => {
                if let Some((value, next)) = read_varint(data, index) {
                    scan.varints.push(VarintField {
                        path: field_path,
                        value,
                    });
                    index = next;
                } else {
                    index = start + 1;
                }
            }
            1 => {
                if index + 8 > data.len() {
                    break;
                }
                index += 8;
            }
            2 => {
                let Some((len, next)) = read_varint(data, index) else {
                    index = start + 1;
                    continue;
                };
                index = next;
                let end = index.saturating_add(len as usize);
                if end > data.len() {
                    break;
                }
                if depth < 4 {
                    let nested = scan_protobuf(&data[index..end], depth + 1, &field_path);
                    scan.fixed32.extend(nested.fixed32);
                    scan.varints.extend(nested.varints);
                }
                index = end;
            }
            5 => {
                if index + 4 > data.len() {
                    break;
                }
                let bits = u32::from_le_bytes(data[index..index + 4].try_into().unwrap_or([0; 4]));
                let value = f32::from_bits(bits);
                scan.fixed32.push(Fixed32Field {
                    path: field_path,
                    value,
                    order,
                });
                order += 1;
                index += 4;
            }
            _ => {
                index = start + 1;
            }
        }
    }
    scan
}

fn read_varint(data: &[u8], mut index: usize) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    while index < data.len() && shift < 64 {
        let byte = data[index];
        index += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index));
        }
        shift += 7;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grpc_frame(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8];
        let len = payload.len() as u32;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn varint(mut value: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            bytes.push(byte);
            if value == 0 {
                break;
            }
        }
        bytes
    }

    fn encode_timestamp(seconds: u64) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.push(0x08); // field 1 varint
        msg.extend(varint(seconds));
        msg
    }

    fn encode_product(id: u32, percent: f32) -> Vec<u8> {
        let mut msg = Vec::new();
        msg.push(0x08);
        msg.extend(varint(id as u64));
        msg.push(0x15); // field 2 fixed32
        msg.extend_from_slice(&percent.to_le_bytes());
        msg
    }

    fn sample_payload(used: f32, start: u64, reset: u64) -> Vec<u8> {
        let mut inner = Vec::new();
        // field 1 fixed32 used percent
        inner.push(0x0d);
        inner.extend_from_slice(&used.to_le_bytes());
        // field 4 timestamp
        let ts_start = encode_timestamp(start);
        inner.push(0x22);
        inner.extend(varint(ts_start.len() as u64));
        inner.extend(ts_start);
        // field 5 timestamp
        let ts_reset = encode_timestamp(reset);
        inner.push(0x2a);
        inner.extend(varint(ts_reset.len() as u64));
        inner.extend(ts_reset);
        // products
        for (id, pct) in [(1u32, 50.0f32), (2u32, 22.0f32)] {
            let p = encode_product(id, pct);
            inner.push(0x3a); // field 7
            inner.extend(varint(p.len() as u64));
            inner.extend(p);
        }
        let mut payload = Vec::new();
        payload.push(0x0a); // field 1 message
        payload.extend(varint(inner.len() as u64));
        payload.extend(inner);
        payload
    }

    #[test]
    fn parses_structured_billing_payload() {
        let payload = sample_payload(72.0, 1_783_711_988, 1_784_316_788);
        let framed = grpc_frame(&payload);
        // trailer success
        let mut full = framed;
        let trailer = b"grpc-status:0\r\n";
        full.push(0x80);
        full.extend_from_slice(&(trailer.len() as u32).to_be_bytes());
        full.extend_from_slice(trailer);

        let snap = parse_grpc_web_quota(&full).expect("parse");
        assert!((snap.used_percent - 72.0).abs() < 0.01);
        assert!((snap.remaining_percent - 28.0).abs() < 0.01);
        assert_eq!(snap.products.len(), 2);
        assert_eq!(snap.products[0].label, "API");
        assert!((snap.products[0].used_percent - 50.0).abs() < 0.01);
        assert_eq!(snap.products[1].label, "Grok Build");
        assert!(snap.resets_at.is_some());
        assert!(snap.period_start_at.is_some());
    }

    #[test]
    fn parses_real_captured_hex() {
        // Captured from live account (2026-07-12): 72% used, reset 2026-07-17T19:33:08Z
        let hex = "0a5d0d0000904212001a00220b08f491c5d20610a896a2772a0b08f486ead20610a896a2773a07080115000048423a070802150000b0413a020804421c0802120b08f491c5d20610a896a2771a0b08f486ead20610a896a277580162006801";
        let payload = hex::decode_like(hex);
        let snap = parse_grpc_web_quota(&payload).expect("parse live sample");
        assert!((snap.used_percent - 72.0).abs() < 0.01);
        assert_eq!(
            snap.resets_at.unwrap().timestamp(),
            1_784_316_788
        );
        assert!(snap.products.len() >= 2);
        assert!((snap.products[0].used_percent - 50.0).abs() < 0.01);
        assert!((snap.products[1].used_percent - 22.0).abs() < 0.01);
    }

    // tiny hex helper without extra crate
    mod hex {
        pub fn decode_like(s: &str) -> Vec<u8> {
            let s = s.trim();
            let mut out = Vec::with_capacity(s.len() / 2);
            let bytes = s.as_bytes();
            let mut i = 0;
            while i + 1 < bytes.len() {
                let h = from_hex(bytes[i]);
                let l = from_hex(bytes[i + 1]);
                out.push((h << 4) | l);
                i += 2;
            }
            out
        }
        fn from_hex(b: u8) -> u8 {
            match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => 0,
            }
        }
    }
}
