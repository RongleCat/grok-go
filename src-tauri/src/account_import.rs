//! Multi-format account credential import (CPA / sub2api / 卡网 SSO paste).
//!
//! Supported payloads:
//! - Card-seller lines with any common separator, e.g.
//!   `email----password----SSO`, `email|password|SSO`, or free text that embeds a
//!   web SSO JWT (`eyJ…` with `session_id` claim). JWT is matched by shape, not by
//!   a fixed delimiter layout.
//! - Pure SSO JWT list (session_id payload) or `sso=<jwt>` (same conversion)
//! - Plain OAuth refresh tokens: one per line (sub2api Grok RT batch)
//! - CPA xAI auth file JSON: `{"type":"xai","access_token","refresh_token",...}`
//! - Array of CPA files, NDJSON (one JSON object per line)
//! - sub2api credentials / account objects (with nested `credentials`)
//! - GrokGo `auth.json` shape: `{"accounts":[...]}`
//!
//! Card SSO is **not** used for grok.com reverse chat; import runs OIDC Device Flow
//! with the SSO cookie and stores official access/refresh tokens.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::config::{Account, AccountAuthKind, AccountHealth, SsoPoolTier};

/// One parsed credential ready to become an [`Account`].
#[derive(Debug, Clone)]
pub struct ParsedCredential {
    pub name: Option<String>,
    pub email: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub sso_token: Option<String>,
    pub password: Option<String>,
    pub auth_kind: AccountAuthKind,
    pub token_type: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub source_format: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAccountsOptions {
    /// Default weight for newly created accounts.
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default = "default_true")]
    pub supports_image: bool,
    #[serde(default = "default_true")]
    pub supports_video: bool,
    /// Skip entries whose refresh_token or email already exists.
    #[serde(default = "default_true")]
    pub skip_duplicates: bool,
    /// Refresh RT-only entries to obtain access_token (network). Default true.
    #[serde(default = "default_true")]
    pub validate_refresh: bool,
}

fn default_weight() -> u32 {
    1
}
fn default_true() -> bool {
    true
}

impl Default for ImportAccountsOptions {
    fn default() -> Self {
        Self {
            weight: 1,
            supports_image: true,
            supports_video: true,
            skip_duplicates: true,
            validate_refresh: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAccountsResult {
    pub added: usize,
    pub skipped: usize,
    pub failed: usize,
    pub accounts: Vec<Account>,
    pub errors: Vec<ImportErrorItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportErrorItem {
    pub index: usize,
    pub detail: String,
}

/// Parse free-form text/JSON into credential candidates (no network).
pub fn parse_import_payload(raw: &str) -> Result<Vec<ParsedCredential>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("import payload is empty".into());
    }

    // Prefer whole-document JSON when it looks like JSON.
    if looks_like_json(trimmed) {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            let mut out = Vec::new();
            collect_from_value(&value, &mut out);
            if !out.is_empty() {
                return Ok(out);
            }
            return Err("JSON parsed but no Grok/xAI credentials found".into());
        }
        // Mixed NDJSON or broken single object — try line-by-line JSON then RT fallback.
        let mut out = Vec::new();
        let mut json_lines = 0usize;
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if looks_like_json(line) {
                json_lines += 1;
                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    collect_from_value(&value, &mut out);
                }
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
        if json_lines > 0 {
            return Err("JSON lines parsed but no usable credentials found".into());
        }
    }

    // Card / free-form lines: SSO JWT may be embedded with any separator
    // (----, |, whitespace, …). Also bare SSO JWT / RT lists.
    let lines = parse_plain_lines(trimmed);
    if lines.is_empty() {
        return Err("no refresh tokens, SSO tokens, or credential JSON found".into());
    }
    Ok(lines)
}

/// Parse free-form text lines into credentials (card SSO / RT / bare JWT).
fn parse_plain_lines(raw: &str) -> Vec<ParsedCredential> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim().trim_end_matches(['；', ';']).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Prefer lines that embed a JWT (any separator layout).
        if let Some(cred) = parse_line_with_sso_jwt(line) {
            out.push(cred);
            continue;
        }
        let token = line
            .strip_prefix("sso=")
            .or_else(|| line.strip_prefix("SSO="))
            .unwrap_or(line)
            .trim()
            .trim_matches(|c| c == '"' || c == '\'' || c == '`' || c == ';');
        if token.len() < 20 {
            continue;
        }
        if is_web_sso_jwt(token) {
            out.push(ParsedCredential {
                name: None,
                email: None,
                access_token: None,
                refresh_token: None,
                sso_token: Some(token.to_string()),
                password: None,
                auth_kind: AccountAuthKind::Sso,
                token_type: None,
                expires_at: None,
                notes: None,
                source_format: "sso-jwt-list",
            });
        } else if looks_like_refresh_token(token) {
            out.push(ParsedCredential {
                name: None,
                email: None,
                access_token: None,
                refresh_token: Some(token.to_string()),
                sso_token: None,
                password: None,
                auth_kind: AccountAuthKind::Oauth,
                token_type: Some("Bearer".into()),
                expires_at: None,
                notes: None,
                source_format: "refresh-token-list",
            });
        }
    }
    out
}

/// Scan a free-form card line for an embedded web SSO JWT, plus optional email/password.
///
/// Accepts layouts such as:
/// - `email----password----eyJ…`
/// - `email|password|eyJ…`
/// - `email password eyJ…`
/// - bare `eyJ…` (handled by caller as list)
/// - noisy seller pastes with instruction text on other lines
fn parse_line_with_sso_jwt(line: &str) -> Option<ParsedCredential> {
    let sso = find_jwt_in_text(line)?;
    // Prefer true web-SSO (session_id). Still accept long eyJ tokens as SSO candidates
    // when sellers ship non-standard claims.
    if !is_web_sso_jwt(sso) && !(sso.starts_with("eyJ") && sso.len() >= 40) {
        return None;
    }

    // Everything before the JWT is email/password (and optional labels).
    let before = line.get(..line.find(sso)?)?.trim();
    let before = before
        .trim_end_matches(['|', '-', ':', '=', ',', ';', ' ', '\t'])
        .trim();
    let before = before
        .strip_suffix("sso")
        .or_else(|| before.strip_suffix("SSO"))
        .unwrap_or(before)
        .trim_end_matches(['|', '-', ':', '=', ',', ';', ' ', '\t'])
        .trim();

    let (email, password) = split_email_password_prefix(before);
    let email = email.filter(|e| e.contains('@'));

    let source = if email.is_some() && password.is_some() {
        "card-email-password-sso"
    } else if email.is_some() {
        "card-email-sso"
    } else {
        "sso-jwt-embedded"
    };

    Some(ParsedCredential {
        name: email.clone(),
        email,
        access_token: None,
        refresh_token: None,
        sso_token: Some(sso.to_string()),
        password,
        auth_kind: AccountAuthKind::Sso,
        token_type: None,
        expires_at: None,
        notes: Some(format!("card-sso import ({})", Utc::now().format("%Y-%m-%d"))),
        source_format: source,
    })
}

/// Split `email|password`, `email----password`, or lone email from the prefix before JWT.
fn split_email_password_prefix(prefix: &str) -> (Option<String>, Option<String>) {
    let p = prefix.trim();
    if p.is_empty() {
        return (None, None);
    }

    // Prefer multi-char then single-char delimiters.
    let parts: Vec<&str> = if p.contains("----") {
        p.split("----").map(str::trim).filter(|s| !s.is_empty()).collect()
    } else if p.contains('|') {
        p.split('|').map(str::trim).filter(|s| !s.is_empty()).collect()
    } else if p.contains('\t') {
        p.split('\t').map(str::trim).filter(|s| !s.is_empty()).collect()
    } else if p.contains(" - ") {
        p.split(" - ").map(str::trim).filter(|s| !s.is_empty()).collect()
    } else {
        // whitespace-separated email password
        p.split_whitespace().filter(|s| !s.is_empty()).collect()
    };

    if parts.is_empty() {
        return (None, None);
    }
    if parts.len() == 1 {
        return (Some(parts[0].to_string()), None);
    }
    // First field with @ is email; next non-empty is password.
    let email_idx = parts.iter().position(|s| s.contains('@')).unwrap_or(0);
    let email = parts[email_idx].to_string();
    let password = parts
        .iter()
        .enumerate()
        .filter(|(i, s)| *i != email_idx && !s.is_empty() && !s.starts_with("eyJ"))
        .map(|(_, s)| (*s).to_string())
        .next();
    (Some(email), password)
}

/// Locate the first JWT-shaped token (`header.payload.signature`, base64url) in text.
/// Does not require a fixed delimiter — works for `|`, `----`, spaces, or glue.
fn find_jwt_in_text(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 20 < bytes.len() {
        // JWT headers typically start with base64url("{"...) → "eyJ"
        if bytes[i] == b'e'
            && bytes.get(i + 1) == Some(&b'y')
            && bytes.get(i + 2) == Some(&b'J')
        {
            let start = i;
            let mut dots = 0u8;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if c == b'.' {
                    dots = dots.saturating_add(1);
                    j += 1;
                    continue;
                }
                if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                    j += 1;
                    continue;
                }
                break;
            }
            // Standard compact JWT: exactly 2 dots, reasonable length.
            if dots == 2 && j.saturating_sub(start) >= 40 {
                return Some(&s[start..j]);
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    None
}

/// Refresh tokens are single-token lines — not free prose or emails.
fn looks_like_refresh_token(s: &str) -> bool {
    let s = s.trim();
    if s.len() < 24 {
        return false;
    }
    if s.starts_with("rt_") {
        return s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    }
    // Avoid absorbing instruction text / Chinese seller notes / URLs.
    if s.contains('@')
        || s.contains(' ')
        || s.contains("http")
        || s.chars().any(|c| !c.is_ascii())
    {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+'))
}

/// Detect grok.com web SSO JWT (payload has session_id, not OAuth refresh).
pub fn is_web_sso_jwt(token: &str) -> bool {
    let token = token.trim().strip_prefix("sso=").unwrap_or(token.trim());
    if !token.starts_with("eyJ") {
        return false;
    }
    let mut parts = token.split('.');
    let _hdr = match parts.next() {
        Some(h) => h,
        None => return false,
    };
    let payload_b64 = match parts.next() {
        Some(p) => p,
        None => return false,
    };
    let payload = match decode_jwt_payload(payload_b64) {
        Some(p) => p,
        None => return false,
    };
    // Card-seller SSO: {"session_id":"uuid"}
    if payload.get("session_id").and_then(|v| v.as_str()).is_some() {
        return true;
    }
    // Not a normal OAuth access token if it lacks typical claims.
    let has_oauth = payload.get("scope").is_some()
        || payload.get("client_id").is_some()
        || payload.get("aud").is_some();
    !has_oauth && payload.get("sub").is_none()
}

fn decode_jwt_payload(payload_b64: &str) -> Option<Value> {
    use base64::Engine;
    let mut s = payload_b64.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(s).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn looks_like_json(s: &str) -> bool {
    let s = s.trim_start();
    s.starts_with('{') || s.starts_with('[')
}

fn collect_from_value(value: &Value, out: &mut Vec<ParsedCredential>) {
    match value {
        Value::Array(arr) => {
            for item in arr {
                collect_from_value(item, out);
            }
        }
        Value::Object(map) => {
            // GrokGo auth store
            if let Some(accounts) = map.get("accounts").and_then(|v| v.as_array()) {
                for item in accounts {
                    collect_from_value(item, out);
                }
                return;
            }
            // sub2api nested credentials
            if let Some(creds) = map.get("credentials") {
                let mut nested = Vec::new();
                collect_from_value(creds, &mut nested);
                for mut c in nested {
                    if c.name.is_none() {
                        c.name = string_field(map, &["name", "label", "title"]);
                    }
                    if c.email.is_none() {
                        c.email = string_field(map, &["email"])
                            .or_else(|| string_field_from_extra(map));
                    }
                    if c.notes.is_none() {
                        c.notes = string_field(map, &["notes", "note", "remark"]);
                    }
                    out.push(c);
                }
                // Also pull top-level tokens if credentials had none.
                if let Some(parsed) = try_parse_credential_object(map) {
                    out.push(parsed);
                }
                return;
            }
            if let Some(parsed) = try_parse_credential_object(map) {
                out.push(parsed);
                return;
            }
            // Recurse into common wrappers (sso, data, token, auth, session).
            for key in [
                "sso",
                "data",
                "token",
                "tokens",
                "auth",
                "session",
                "oauth",
                "credential",
                "items",
                "list",
            ] {
                if let Some(inner) = map.get(key) {
                    collect_from_value(inner, out);
                }
            }
        }
        Value::String(s) => {
            let s = s.trim().strip_prefix("sso=").unwrap_or(s.trim());
            if s.len() < 20 {
                return;
            }
            if is_web_sso_jwt(s) {
                out.push(ParsedCredential {
                    name: None,
                    email: None,
                    access_token: None,
                    refresh_token: None,
                    sso_token: Some(s.to_string()),
                    password: None,
                    auth_kind: AccountAuthKind::Sso,
                    token_type: None,
                    expires_at: None,
                    notes: None,
                    source_format: "json-string-sso",
                });
            } else {
                out.push(ParsedCredential {
                    name: None,
                    email: None,
                    access_token: None,
                    refresh_token: Some(s.to_string()),
                    sso_token: None,
                    password: None,
                    auth_kind: AccountAuthKind::Oauth,
                    token_type: Some("Bearer".into()),
                    expires_at: None,
                    notes: None,
                    source_format: "json-string-token",
                });
            }
        }
        _ => {}
    }
}

fn string_field_from_extra(map: &serde_json::Map<String, Value>) -> Option<String> {
    map.get("extra")
        .and_then(|v| v.as_object())
        .and_then(|extra| string_field(extra, &["email", "email_address"]))
}

fn try_parse_credential_object(
    map: &serde_json::Map<String, Value>,
) -> Option<ParsedCredential> {
    let mut sso = string_field(map, &["sso_token", "ssoToken", "sso"]);
    let access = string_field(
        map,
        &[
            "access_token",
            "accessToken",
            "token",
            "api_key",
            "apiKey",
        ],
    );
    let refresh = string_field(map, &["refresh_token", "refreshToken", "rt", "RT"]);

    // Promote token/access that looks like web SSO JWT.
    if sso.is_none() {
        if let Some(ref a) = access {
            if is_web_sso_jwt(a) {
                sso = Some(a.clone());
            }
        }
    }

    let auth_kind = if sso.is_some()
        || string_field(map, &["auth_kind", "authKind", "type"])
            .map(|t| {
                let t = t.to_ascii_lowercase();
                t == "sso" || t.contains("sso")
            })
            .unwrap_or(false)
    {
        AccountAuthKind::Sso
    } else {
        AccountAuthKind::Oauth
    };

    // Must have at least one usable secret.
    let has_secret = sso.as_ref().map(|s| s.len() >= 20).unwrap_or(false)
        || access.as_ref().map(|s| s.len() >= 10).unwrap_or(false)
        || refresh.as_ref().map(|s| s.len() >= 10).unwrap_or(false);
    if !has_secret {
        return None;
    }

    // Skip non-xAI CPA types when type is explicitly set to something else.
    if let Some(ty) = string_field(map, &["type", "provider", "platform"]) {
        let ty = ty.to_ascii_lowercase();
        if !matches!(
            ty.as_str(),
            "xai" | "grok" | "x-ai" | "oauth" | "sso" | "" | "unknown"
        ) && refresh.is_none()
            && sso.is_none()
            && !ty.contains("grok")
            && !ty.contains("xai")
            && !ty.contains("sso")
        {
            if string_field(map, &["base_url", "baseUrl"])
                .map(|u| !u.contains("x.ai") && !u.contains("grok.com"))
                .unwrap_or(true)
                && !map.contains_key("access_token")
                && !map.contains_key("refresh_token")
                && !map.contains_key("sso_token")
            {
                return None;
            }
        }
    }

    let email = string_field(map, &["email", "Email"])
        .or_else(|| string_field(map, &["sub"]).filter(|s| s.contains('@')));
    let name = string_field(map, &["name", "label", "Label", "title"])
        .or_else(|| email.clone());
    let token_type = string_field(map, &["token_type", "tokenType"])
        .or_else(|| {
            if auth_kind == AccountAuthKind::Oauth {
                Some("Bearer".into())
            } else {
                None
            }
        });
    let expires_at = parse_expires(map);
    let notes = string_field(map, &["notes", "note", "remark"]);
    let password = string_field(map, &["password", "password_hint", "passwordHint"]);

    let source = if auth_kind == AccountAuthKind::Sso {
        "sso-json"
    } else if string_field(map, &["type"])
        .map(|t| t.eq_ignore_ascii_case("xai"))
        .unwrap_or(false)
    {
        "cpa-xai-json"
    } else if map.contains_key("auth_kind") || map.contains_key("expired") {
        "cpa-auth-file"
    } else if map.contains_key("entitlement_status") || map.contains_key("subscription_tier") {
        "sub2api-credentials"
    } else if map.contains_key("id") && map.contains_key("weight") {
        "grok-go-account"
    } else {
        "oauth-json"
    };

    Some(ParsedCredential {
        name,
        email,
        access_token: if auth_kind == AccountAuthKind::Sso {
            None
        } else {
            access
        },
        refresh_token: refresh,
        sso_token: sso,
        password,
        auth_kind,
        token_type,
        expires_at,
        notes,
        source_format: source,
    })
}

fn string_field(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = map.get(*k) {
            if let Some(s) = v.as_str() {
                let s = s.trim();
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

fn parse_expires(map: &serde_json::Map<String, Value>) -> Option<DateTime<Utc>> {
    // RFC3339 string fields
    for k in ["expires_at", "expiresAt", "expired", "expire", "expiry"] {
        if let Some(s) = map.get(k).and_then(|v| v.as_str()) {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s.trim()) {
                return Some(dt.with_timezone(&Utc));
            }
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S") {
                return Some(DateTime::from_naive_utc_and_offset(dt, Utc));
            }
        }
        if let Some(n) = map.get(k).and_then(|v| v.as_i64()) {
            // unix seconds or ms
            let secs = if n > 1_000_000_000_000 {
                n / 1000
            } else {
                n
            };
            return DateTime::from_timestamp(secs, 0);
        }
    }
    // expires_in seconds from now
    if let Some(secs) = map
        .get("expires_in")
        .or_else(|| map.get("expiresIn"))
        .and_then(|v| v.as_u64())
    {
        return Some(Utc::now() + Duration::seconds(secs as i64));
    }
    None
}

/// Build an Account from a parsed credential (tokens as-is; caller may refresh).
pub fn credential_to_account(cred: &ParsedCredential, opts: &ImportAccountsOptions) -> Account {
    let email = cred.email.clone();
    let name = cred
        .name
        .clone()
        .or_else(|| email.clone())
        .unwrap_or_else(|| format!("Imported {}", &Uuid::new_v4().to_string()[..8]));

    let mut account = Account::new(name);
    account.email = email;
    account.enabled = true;
    account.weight = opts.weight.max(1);
    account.auth_kind = cred.auth_kind;
    account.access_token = cred.access_token.clone();
    account.refresh_token = cred.refresh_token.clone();
    account.sso_token = cred.sso_token.clone();
    account.password_hint = cred.password.clone();
    if cred.auth_kind == AccountAuthKind::Sso {
        // Temporary until import converts SSO → OAuth via device flow.
        account.sso_pool = SsoPoolTier::Basic;
        account.token_type = None;
    } else {
        account.token_type = cred
            .token_type
            .clone()
            .or_else(|| Some("Bearer".into()));
    }
    account.expires_at = cred.expires_at;
    account.health = if account.is_credentialed() {
        AccountHealth::Healthy
    } else {
        AccountHealth::Degraded
    };
    account.notes = cred.notes.clone().or_else(|| {
        Some(format!(
            "imported via {} ({})",
            cred.source_format,
            Utc::now().format("%Y-%m-%d %H:%M")
        ))
    });
    account.supports_image = opts.supports_image;
    account.supports_video = opts.supports_video;
    account
}

/// Duplicate detection key: prefer sso_token / refresh_token, else email, else access_token.
pub fn is_duplicate(existing: &[Account], candidate: &Account) -> bool {
    if let Some(sso) = candidate.effective_sso_token() {
        if existing.iter().any(|a| a.effective_sso_token() == Some(sso)) {
            return true;
        }
    }
    if let Some(rt) = candidate
        .refresh_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if existing.iter().any(|a| {
            a.refresh_token
                .as_ref()
                .map(|s| s.trim() == rt)
                .unwrap_or(false)
        }) {
            return true;
        }
    }
    if let Some(email) = candidate
        .email
        .as_ref()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
    {
        if existing.iter().any(|a| {
            a.email
                .as_ref()
                .map(|s| s.trim().to_ascii_lowercase() == email)
                .unwrap_or(false)
        }) {
            return true;
        }
    }
    if let Some(at) = candidate
        .access_token
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if existing.iter().any(|a| {
            a.access_token
                .as_ref()
                .map(|s| s.trim() == at)
                .unwrap_or(false)
        }) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_refresh_token_lines() {
        let raw = "\
# comment
rt_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
rt_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
";
        let list = parse_import_payload(raw).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].source_format, "refresh-token-list");
        assert_eq!(list[0].auth_kind, AccountAuthKind::Oauth);
    }

    #[test]
    fn parse_card_email_password_sso() {
        let sso = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzZXNzaW9uX2lkIjoiMDk5MTdjYTktNTczNi00YTkyLWJhNzMtZTJmZTc0ZGFiOGE1In0.eCwRhzcd4y7IZayqTfWerCeGcZl0_7nv5r6jwGjkiEo";
        let raw = format!("user@example.com----SecretPass1!----{sso}");
        let list = parse_import_payload(&raw).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].auth_kind, AccountAuthKind::Sso);
        assert_eq!(list[0].email.as_deref(), Some("user@example.com"));
        assert_eq!(list[0].password.as_deref(), Some("SecretPass1!"));
        assert_eq!(list[0].sso_token.as_deref(), Some(sso));
        let acc = credential_to_account(&list[0], &ImportAccountsOptions::default());
        assert_eq!(acc.auth_kind, AccountAuthKind::Sso);
        assert_eq!(acc.sso_token.as_deref(), Some(sso));
        // Not routable until import runs SSO→OAuth device flow.
        assert!(!acc.is_credentialed());
    }

    #[test]
    fn parse_card_pipe_separated_sso() {
        let sso = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzZXNzaW9uX2lkIjoiZjlkMWZhN2QtZWU2Ny00MWQ1LWJkZGMtYTYzODNkMWRmOWMxIn0.32YTdvxVYScciAjzNFpWU4L0GohvAWoC5sgz1mL38-Y";
        let raw = format!("uzq4tn@chilloliandfii.space|Grok123123@|{sso}");
        let list = parse_import_payload(&raw).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].email.as_deref(), Some("uzq4tn@chilloliandfii.space"));
        assert_eq!(list[0].password.as_deref(), Some("Grok123123@"));
        assert_eq!(list[0].sso_token.as_deref(), Some(sso));
        assert_eq!(list[0].source_format, "card-email-password-sso");
    }

    #[test]
    fn parse_seller_paste_with_instructions_and_pipe_cards() {
        let sso1 = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzZXNzaW9uX2lkIjoiZjlkMWZhN2QtZWU2Ny00MWQ1LWJkZGMtYTYzODNkMWRmOWMxIn0.32YTdvxVYScciAjzNFpWU4L0GohvAWoC5sgz1mL38-Y";
        let sso2 = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzZXNzaW9uX2lkIjoiNjM3ZGMyZjctMmY4Ni00MWU1LWI1ZDktYzBlYmYwY2E0M2I3In0.b8-L8qdem3BJF5E_KcBU_rbEo-Vmv_wYsG7Z9xVTrco";
        let raw = format!(
            "=== 使用说明 ===\n\
             Super Grok 7天会员 账号格式：邮箱|密码 查看 https://grokcheck.site/\n\
             === 卡密内容 ===\n\
             a@chilloliandfii.space|Grok123123@|{sso1}\n\
             b@tuyenchau.click|Grok123123@|{sso2}\n"
        );
        let list = parse_import_payload(&raw).unwrap();
        assert_eq!(list.len(), 2, "should skip instruction lines, keep 2 cards");
        assert_eq!(list[0].email.as_deref(), Some("a@chilloliandfii.space"));
        assert_eq!(list[1].email.as_deref(), Some("b@tuyenchau.click"));
        assert!(list.iter().all(|c| c.sso_token.is_some()));
    }

    #[test]
    fn find_jwt_ignores_prose() {
        assert!(find_jwt_in_text("no token here email|password only").is_none());
        let sso = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzZXNzaW9uX2lkIjoiZjlkMWZhN2QtZWU2Ny00MWQ1LWJkZGMtYTYzODNkMWRmOWMxIn0.32YTdvxVYScciAjzNFpWU4L0GohvAWoC5sgz1mL38-Y";
        assert_eq!(find_jwt_in_text(&format!("prefix {sso} suffix")), Some(sso));
    }

    #[test]
    fn detect_web_sso_jwt() {
        let sso = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzZXNzaW9uX2lkIjoiMDk5MTdjYTktNTczNi00YTkyLWJhNzMtZTJmZTc0ZGFiOGE1In0.eCwRhzcd4y7IZayqTfWerCeGcZl0_7nv5r6jwGjkiEo";
        assert!(is_web_sso_jwt(sso));
        assert!(is_web_sso_jwt(&format!("sso={sso}")));
        assert!(!is_web_sso_jwt("rt_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    }

    #[test]
    fn parse_cpa_xai_json() {
        let raw = r#"{
          "type": "xai",
          "access_token": "at_abcdefghijklmnopqrstuvwxyz012345",
          "refresh_token": "rt_abcdefghijklmnopqrstuvwxyz012345",
          "email": "user@example.com",
          "expired": "2026-07-13T12:00:00Z",
          "auth_kind": "oauth"
        }"#;
        let list = parse_import_payload(raw).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].email.as_deref(), Some("user@example.com"));
        assert!(list[0].refresh_token.is_some());
        assert_eq!(list[0].source_format, "cpa-xai-json");
    }

    #[test]
    fn parse_cpa_array() {
        let raw = r#"[
          {"type":"xai","refresh_token":"rt_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","email":"a@x.com"},
          {"type":"xai","refresh_token":"rt_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","email":"b@x.com"}
        ]"#;
        let list = parse_import_payload(raw).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn parse_sub2api_credentials() {
        let raw = r#"{
          "name": "Grok OAuth",
          "platform": "grok",
          "credentials": {
            "access_token": "at_abcdefghijklmnopqrstuvwxyz012345",
            "refresh_token": "rt_abcdefghijklmnopqrstuvwxyz012345",
            "email": "g@x.ai",
            "entitlement_status": "active"
          }
        }"#;
        let list = parse_import_payload(raw).unwrap();
        assert!(!list.is_empty());
        assert!(list.iter().any(|c| c.email.as_deref() == Some("g@x.ai")));
    }

    #[test]
    fn parse_grok_go_auth_store() {
        let raw = r#"{
          "accounts": [
            {
              "id": "1",
              "name": "me",
              "enabled": true,
              "weight": 2,
              "accessToken": "at_abcdefghijklmnopqrstuvwxyz012345",
              "refreshToken": "rt_abcdefghijklmnopqrstuvwxyz012345",
              "email": "me@x.ai",
              "consecutiveFailures": 0,
              "health": "healthy"
            }
          ]
        }"#;
        // camelCase accessToken won't match snake_case in try_parse — Account from our store uses serde rename
        // Our parser looks for access_token AND accessToken.
        let list = parse_import_payload(raw).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].email.as_deref(), Some("me@x.ai"));
    }

    #[test]
    fn duplicate_by_refresh_token() {
        let mut a = Account::new("a");
        a.refresh_token = Some("rt_same".into());
        let mut b = Account::new("b");
        b.refresh_token = Some("rt_same".into());
        assert!(is_duplicate(&[a], &b));
    }

    #[test]
    fn credential_to_account_sets_media_flags() {
        let cred = ParsedCredential {
            name: Some("t".into()),
            email: None,
            access_token: Some("at_abcdefghijklmnopqrstuvwxyz012345".into()),
            refresh_token: Some("rt_abcdefghijklmnopqrstuvwxyz012345".into()),
            sso_token: None,
            password: None,
            auth_kind: AccountAuthKind::Oauth,
            token_type: None,
            expires_at: None,
            notes: None,
            source_format: "test",
        };
        let opts = ImportAccountsOptions {
            supports_image: false,
            supports_video: true,
            ..Default::default()
        };
        let acc = credential_to_account(&cred, &opts);
        assert!(!acc.supports_image);
        assert!(acc.supports_video);
    }
}
