//! Shared tool-surface helpers: integer coerce, codex-compat inject set,
//! recent-artifact discovery for long-job recovery.

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Coerce JSON numbers that are whole floats into integers when schema expects integer.
/// Walks object/array trees in-place. Returns true if any value changed.
pub fn coerce_integer_like_numbers(value: &mut Value) -> bool {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 && !n.is_i64() && !n.is_u64() {
                    *value = json!(f as i64);
                    return true;
                }
            }
            false
        }
        Value::Object(map) => {
            let mut changed = false;
            for v in map.values_mut() {
                if coerce_integer_like_numbers(v) {
                    changed = true;
                }
            }
            changed
        }
        Value::Array(arr) => {
            let mut changed = false;
            for v in arr.iter_mut() {
                if coerce_integer_like_numbers(v) {
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
}

/// Coerce known integer-ish MCP tool args (n, duration when whole, session_id-like).
pub fn coerce_mcp_tool_arguments(args: &mut Value) -> bool {
    coerce_integer_like_numbers(args)
}

/// Compact Codex-compat tool types injected on experimental Build plane (stable order).
pub fn codex_compat_inject_tools() -> Vec<Value> {
    vec![
        json!({"type": "x_search"}),
        crate::gateway::image_bridge::image_gen_function_tool(),
    ]
}

/// Whether tools array already has x_search / web_search built-in.
pub fn tools_have_x_search(tools: &[Value]) -> bool {
    tools.iter().any(|t| {
        matches!(
            t.get("type").and_then(|v| v.as_str()),
            Some("x_search" | "web_search")
        )
    })
}

pub fn tools_have_image_gen(tools: &[Value]) -> bool {
    tools.iter().any(|t| {
        let ty = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
        matches!(ty, "image_generation" | "image_gen")
            || crate::gateway::image_bridge::is_image_gen_name(name)
    })
}

/// Inject compact set into an existing tools array (mutates). Returns true if modified.
pub fn inject_codex_compat_tools(tools: &mut Vec<Value>) -> bool {
    let mut modified = false;
    if !tools_have_x_search(tools) {
        tools.push(json!({"type": "x_search"}));
        modified = true;
    }
    if !tools_have_image_gen(tools) {
        tools.push(crate::gateway::image_bridge::image_gen_function_tool());
        modified = true;
    }
    modified
}

/// Ensure `tools` array exists then inject. Returns whether body tools were created/changed.
pub fn ensure_and_inject_codex_compat(value: &mut Value) -> bool {
    let mut modified = false;
    let needs_tools = value
        .get("tools")
        .and_then(|t| t.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(true);
    if needs_tools {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("tools".into(), json!([]));
            modified = true;
        }
    }
    if let Some(tools) = value.get_mut("tools").and_then(|t| t.as_array_mut()) {
        if inject_codex_compat_tools(tools) {
            modified = true;
        }
    }
    modified
}

/// List recent files under artifacts dir (newest first), optionally filtered by extension.
pub fn recent_artifacts(dir: &Path, ext: Option<&str>, limit: usize) -> Vec<String> {
    let mut entries: Vec<(SystemTime, PathBuf)> = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if !path.is_file() {
            continue;
        }
        if let Some(want) = ext {
            let ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case(want))
                .unwrap_or(false);
            if !ok {
                continue;
            }
        }
        let mtime = ent
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        entries.push((mtime, path));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    entries
        .into_iter()
        .take(limit)
        .map(|(_, p)| p.display().to_string())
        .collect()
}

/// Minimum pixel dimension accepted by upstream vision (reject tiny 1×1 probes early).
pub const MIN_IMAGE_SIDE_PX: u32 = 8;

/// Precheck decoded image dimensions from PNG/JPEG/WebP headers when possible.
/// Returns Ok(()) or Err(message with actionable hint).
pub fn precheck_image_bytes(bytes: &[u8]) -> Result<(), String> {
    let (w, h) = match image_dimensions(bytes) {
        Some(d) => d,
        None => return Ok(()), // unknown format — let upstream decide
    };
    if w < MIN_IMAGE_SIDE_PX || h < MIN_IMAGE_SIDE_PX {
        return Err(format!(
            "image too small ({w}x{h}); upstream requires at least {MIN_IMAGE_SIDE_PX}x{MIN_IMAGE_SIDE_PX} pixels. Use a real screenshot or resize the source."
        ));
    }
    Ok(())
}

/// Minimal dimension sniffers (PNG IHDR, JPEG SOF, GIF header) — no full image crate.
fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        // IHDR width/height at offset 16
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((w, h));
    }
    if bytes.len() >= 10 && bytes.starts_with(b"GIF8") {
        let w = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
        let h = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
        return Some((w, h));
    }
    // JPEG: scan for SOF0/SOF2
    if bytes.len() > 4 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        let mut i = 2usize;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if marker == 0xD9 || marker == 0xDA {
                break;
            }
            if i + 4 >= bytes.len() {
                break;
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if matches!(marker, 0xC0 | 0xC1 | 0xC2) && i + 9 < bytes.len() {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return Some((w, h));
            }
            if len < 2 {
                break;
            }
            i += 2 + len;
        }
    }
    None
}

/// Walk OpenAI/Anthropic-ish bodies and precheck base64 data-URL images.
pub fn precheck_vision_in_body(value: &Value) -> Result<(), String> {
    fn walk(v: &Value) -> Result<(), String> {
        match v {
            Value::Object(map) => {
                // OpenAI image_url.url / Anthropic source.data
                if let Some(url) = map
                    .get("url")
                    .or_else(|| map.get("image_url").and_then(|u| u.get("url")))
                    .and_then(|u| u.as_str())
                {
                    if let Some(b64) = data_url_base64(url) {
                        if let Ok(bytes) = base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            b64,
                        ) {
                            precheck_image_bytes(&bytes)?;
                        }
                    }
                }
                if let Some(data) = map.get("data").and_then(|d| d.as_str()) {
                    // Anthropic image source block often has media_type + data
                    if map.get("media_type").is_some() || map.get("type").and_then(|t| t.as_str()) == Some("base64") {
                        if let Ok(bytes) =
                            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                        {
                            precheck_image_bytes(&bytes)?;
                        }
                    }
                }
                for child in map.values() {
                    walk(child)?;
                }
                Ok(())
            }
            Value::Array(arr) => {
                for child in arr {
                    walk(child)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    walk(value)
}

fn data_url_base64(url: &str) -> Option<&str> {
    let url = url.trim();
    if !url.starts_with("data:") {
        return None;
    }
    let comma = url.find(',')?;
    Some(&url[comma + 1..])
}

/// Observability header names (lowercase for HeaderMap).
pub const HDR_PLANE: &str = "x-grokgo-plane";
pub const HDR_ACCOUNT: &str = "x-grokgo-account";
pub const HDR_UPSTREAM_MS: &str = "x-grokgo-upstream-ms";
pub const HDR_TRUNCATED: &str = "x-grokgo-truncated";
pub const HDR_THINKING: &str = "x-grokgo-thinking-mode";
pub const HDR_TOKEN_COUNT_MODE: &str = "x-grokgo-token-count-mode";
pub const HDR_CACHE_MODE: &str = "x-grokgo-cache-mode";
/// Comma-separated tools injected this request (experimental Build), e.g. `x_search,image_gen`.
pub const HDR_TOOLS_INJECTED: &str = "x-grokgo-tools-injected";
pub const HDR_MODEL_REQUESTED: &str = "x-grokgo-model-requested";
pub const HDR_MODEL_ROUTED: &str = "x-grokgo-model-routed";
pub const HDR_MODEL_UPSTREAM: &str = "x-grokgo-model-upstream";
pub const HDR_CONVERT_MS: &str = "x-grokgo-convert-ms";
pub const HDR_OPTIMIZE_MS: &str = "x-grokgo-optimize-ms";

/// Short stable hash of account id for response headers (not full secret).
pub fn short_account_tag(account_id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    account_id.hash(&mut h);
    format!("{:08x}", (h.finish() as u32))
}

/// Plane label for headers/logs.
pub fn plane_label(build_plane: bool, experimental: bool, media: bool) -> &'static str {
    if media {
        "console-media"
    } else if experimental {
        "experimental-build"
    } else if build_plane {
        "native-build"
    } else {
        "console"
    }
}

/// Flatten object args helper for tools HTTP (accept raw object as body).
pub fn arguments_map_from_body(body: Value) -> Map<String, Value> {
    match body {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("value".into(), other);
            m
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerces_whole_float_to_int() {
        let mut v = json!({"n": 2.0, "session_id": 60619.0, "q": "x", "nested": {"d": 3.5}});
        assert!(coerce_mcp_tool_arguments(&mut v));
        assert_eq!(v["n"], json!(2));
        assert_eq!(v["session_id"], json!(60619));
        assert_eq!(v["nested"]["d"], json!(3.5));
    }

    #[test]
    fn inject_adds_x_search_and_image_gen_only_once() {
        let mut tools = vec![];
        assert!(inject_codex_compat_tools(&mut tools));
        assert!(tools_have_x_search(&tools));
        assert!(tools_have_image_gen(&tools));
        assert!(!inject_codex_compat_tools(&mut tools));
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn experimental_body_gets_tools() {
        let mut body = json!({"model": "grok-4.5", "input": "hi"});
        assert!(ensure_and_inject_codex_compat(&mut body));
        let tools = body["tools"].as_array().unwrap();
        assert!(tools_have_x_search(tools));
        assert!(tools_have_image_gen(tools));
    }

    #[test]
    fn rejects_tiny_png() {
        // 1x1 PNG
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
        let err = precheck_image_bytes(&bytes).unwrap_err();
        assert!(err.contains("too small"));
    }

    #[test]
    fn accepts_32x32_png() {
        // Minimal valid 32x32 is heavy; craft IHDR only check via synthetic PNG header.
        let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        bytes.extend_from_slice(&[0, 0, 0, 13]); // length
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&32u32.to_be_bytes());
        bytes.extend_from_slice(&32u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0]); // bit depth etc
        // CRC placeholder
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        assert!(precheck_image_bytes(&bytes).is_ok());
    }

}
