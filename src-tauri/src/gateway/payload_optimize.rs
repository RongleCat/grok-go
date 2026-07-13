//! Shrink multi-turn Responses / chat payloads before they hit xAI.
//!
//! ## Why this exists
//!
//! Codex agent loops re-send the **entire** conversation every turn (xAI does
//! not use OpenAI-style `previous_response_id` store semantics through this
//! proxy). When the agent reads many large files or attaches multiple images
//! as base64, input tokens and HTTP body size grow linearly → rate limits,
//! timeouts, or hard stops. Restarting the same thread reloads the same bloat
//! (goal mode cannot fix a poisoned session history).
//!
//! ## Strategies (from xAI docs + CPA / sub2api research)
//!
//! 1. **Files API offload**: large text blobs → `POST /v1/files` → `input_file`
//!    `{ file_id }` so document content is processed via `attachment_search`
//!    instead of sitting in every prompt (xAI: content does not reappear fully
//!    in message history).
//! 2. **Image dedupe / collapse**: identical `data:` URLs kept once; older
//!    historical images beyond a budget become short text stubs.
//! 3. **`store: false` when images present**: xAI warns that storing
//!    request/response history with images can make subsequent requests fail.
//! 4. **Historical tool-output truncation**: keep recent full tool results;
//!    older huge `function_call_output` / `input_text` get head+tail stubs.
//! 5. **Body soft/hard budget**: progressive pruning if still oversized.
//!
//! CPA notes `file_id` is **not** supported on xAI image/video *generation*
//! endpoints — vision still uses `input_image` + url/data URL.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use crate::gateway::files_api::{self, DEFAULT_OFFLOAD_TTL_SECS};
use crate::error::AppResult;

/// Soft target for outbound JSON body size (after optimize).
pub const SOFT_BODY_BUDGET: usize = 12 * 1024 * 1024;
/// Hard ceiling — more aggressive prune if still above this.
pub const HARD_BODY_BUDGET: usize = 24 * 1024 * 1024;

/// Keep this many most-recent full images; older ones may collapse.
pub const MAX_FULL_IMAGES: usize = 8;
/// Characters above which a *historical* text/tool blob is truncated.
pub const HISTORICAL_TEXT_SOFT: usize = 12_000;
/// Characters above which we prefer Files API offload (recent or historical).
pub const OFFLOAD_TEXT_MIN: usize = 32_000;
/// Minimum data-URL size to consider for dedupe / collapse.
pub const MIN_DATA_URL_CHARS: usize = 2_000;
/// Truncation head/tail when stubbing large text.
const TRUNC_HEAD: usize = 4_000;
const TRUNC_TAIL: usize = 1_500;

#[derive(Debug, Default, Clone)]
pub struct OptimizeStats {
    pub modified: bool,
    pub original_bytes: usize,
    pub optimized_bytes: usize,
    pub images_deduped: usize,
    pub images_collapsed: usize,
    pub texts_truncated: usize,
    pub files_offloaded: usize,
    pub store_forced_false: bool,
}

impl OptimizeStats {
    pub fn log_summary(&self, path: &str) {
        if !self.modified && self.files_offloaded == 0 {
            return;
        }
        tracing::info!(
            path,
            original_kb = self.original_bytes / 1024,
            optimized_kb = self.optimized_bytes / 1024,
            images_deduped = self.images_deduped,
            images_collapsed = self.images_collapsed,
            texts_truncated = self.texts_truncated,
            files_offloaded = self.files_offloaded,
            store_forced_false = self.store_forced_false,
            "payload optimized before upstream"
        );
    }
}

/// Sync-only optimizations (no network). Safe to call before account selection.
pub fn optimize_responses_payload(value: &mut Value) -> OptimizeStats {
    let original_bytes = value.to_string().len();
    let mut stats = OptimizeStats {
        original_bytes,
        optimized_bytes: original_bytes,
        ..Default::default()
    };

    let has_images = payload_has_images(value);
    if has_images {
        if force_store_false(value) {
            stats.modified = true;
            stats.store_forced_false = true;
        }
    }

    if let Some(input) = value.get_mut("input") {
        optimize_input_array(input, &mut stats, /*aggressive*/ false);
    }
    if let Some(messages) = value.get_mut("messages") {
        optimize_chat_messages(messages, &mut stats, false);
    }

    // Soft budget second pass.
    let size = value.to_string().len();
    if size > SOFT_BODY_BUDGET {
        if let Some(input) = value.get_mut("input") {
            optimize_input_array(input, &mut stats, true);
        }
        if let Some(messages) = value.get_mut("messages") {
            optimize_chat_messages(messages, &mut stats, true);
        }
    }
    // Hard budget: strip remaining large data URLs from non-trailing items.
    let size = value.to_string().len();
    if size > HARD_BODY_BUDGET {
        if let Some(input) = value.get_mut("input") {
            hard_strip_old_media(input, &mut stats);
        }
    }

    stats.optimized_bytes = value.to_string().len();
    if stats.optimized_bytes != stats.original_bytes {
        stats.modified = true;
    }
    stats
}

/// After account selection: offload huge text blobs to xAI Files API and
/// replace them with `input_file` references (+ short text stub).
pub async fn offload_large_text_blobs(
    value: &mut Value,
    client: &reqwest::Client,
    xai_base_url: &str,
    token: &str,
    account_id: &str,
) -> AppResult<OptimizeStats> {
    let original_bytes = value.to_string().len();
    let mut stats = OptimizeStats {
        original_bytes,
        optimized_bytes: original_bytes,
        ..Default::default()
    };

    let Some(Value::Array(items)) = value.get_mut("input") else {
        stats.optimized_bytes = value.to_string().len();
        return Ok(stats);
    };

    // Process from oldest so we free budget first; skip tiny blobs.
    let mut pending_file_parts: Vec<(usize, String, String)> = Vec::new(); // (item_idx, file_id, note)

    for (idx, item) in items.iter_mut().enumerate() {
        // function_call_output.output (string)
        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if ty == "function_call_output" || ty == "custom_tool_call_output" {
            if let Some(output) = item.get("output").and_then(|v| v.as_str()) {
                if output.len() >= OFFLOAD_TEXT_MIN && !output.starts_with("data:") {
                    let bytes = output.as_bytes().to_vec();
                    let filename = format!("tool-output-{idx}.txt");
                    match files_api::upload_cached(
                        client,
                        xai_base_url,
                        token,
                        account_id,
                        &filename,
                        bytes,
                        Some(DEFAULT_OFFLOAD_TTL_SECS),
                    )
                    .await
                    {
                        Ok(file_id) => {
                            let note = format!(
                                "[content offloaded to xAI file {file_id}; full text available via attachment_search / input_file]"
                            );
                            if let Some(obj) = item.as_object_mut() {
                                obj.insert("output".into(), Value::String(note.clone()));
                            }
                            pending_file_parts.push((idx, file_id, note));
                            stats.files_offloaded += 1;
                            stats.modified = true;
                        }
                        Err(err) => {
                            tracing::warn!(
                                account = %account_id,
                                "files offload failed (will truncate instead): {err}"
                            );
                            // Fallback truncate
                            if let Some(obj) = item.as_object_mut() {
                                if let Some(s) = obj.get("output").and_then(|v| v.as_str()) {
                                    let stub = truncate_text(s, TRUNC_HEAD, TRUNC_TAIL);
                                    obj.insert("output".into(), Value::String(stub));
                                    stats.texts_truncated += 1;
                                    stats.modified = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Message content parts with huge input_text
        if let Some(content) = item.get_mut("content").and_then(|c| c.as_array_mut()) {
            for part in content.iter_mut() {
                let pty = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if pty != "input_text" && pty != "text" && pty != "output_text" {
                    continue;
                }
                let Some(text) = part.get("text").and_then(|t| t.as_str()) else {
                    continue;
                };
                if text.len() < OFFLOAD_TEXT_MIN || text.starts_with("data:") {
                    continue;
                }
                let bytes = text.as_bytes().to_vec();
                let filename = format!("input-text-{idx}.txt");
                match files_api::upload_cached(
                    client,
                    xai_base_url,
                    token,
                    account_id,
                    &filename,
                    bytes,
                    Some(DEFAULT_OFFLOAD_TTL_SECS),
                )
                .await
                {
                    Ok(file_id) => {
                        if let Some(obj) = part.as_object_mut() {
                            obj.insert(
                                "text".into(),
                                Value::String(format!(
                                    "[large text offloaded to file {file_id}]"
                                )),
                            );
                        }
                        pending_file_parts.push((
                            idx,
                            file_id,
                            format!("offloaded input text from item {idx}"),
                        ));
                        stats.files_offloaded += 1;
                        stats.modified = true;
                    }
                    Err(err) => {
                        tracing::warn!("files offload input_text failed: {err}");
                        if let Some(obj) = part.as_object_mut() {
                            if let Some(s) = obj.get("text").and_then(|v| v.as_str()) {
                                obj.insert(
                                    "text".into(),
                                    Value::String(truncate_text(s, TRUNC_HEAD, TRUNC_TAIL)),
                                );
                                stats.texts_truncated += 1;
                                stats.modified = true;
                            }
                        }
                    }
                }
            }
        }
    }

    // Attach input_file parts so the model can still retrieve full content.
    // Insert as a synthetic user message after the last offloaded item for clarity.
    if !pending_file_parts.is_empty() {
        let mut file_content = Vec::new();
        file_content.push(json!({
            "type": "input_text",
            "text": format!(
                "The following {} large document(s) were offloaded to xAI Files to reduce multi-turn token usage. Use attachment_search / read them as needed:",
                pending_file_parts.len()
            )
        }));
        for (_idx, file_id, note) in &pending_file_parts {
            file_content.push(json!({
                "type": "input_file",
                "file_id": file_id,
            }));
            file_content.push(json!({
                "type": "input_text",
                "text": note,
            }));
        }
        items.push(json!({
            "role": "user",
            "content": file_content
        }));
        stats.modified = true;
    }

    stats.optimized_bytes = value.to_string().len();
    Ok(stats)
}

fn force_store_false(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    match obj.get("store") {
        Some(Value::Bool(false)) => false,
        _ => {
            obj.insert("store".into(), Value::Bool(false));
            true
        }
    }
}

fn payload_has_images(value: &Value) -> bool {
    walk_find_image(value)
}

fn walk_find_image(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            if let Some(ty) = map.get("type").and_then(|t| t.as_str()) {
                if matches!(ty, "input_image" | "output_image" | "image_url") {
                    return true;
                }
            }
            if let Some(url) = map.get("image_url") {
                if let Some(s) = url.as_str() {
                    if s.starts_with("data:image") {
                        return true;
                    }
                }
                if let Some(inner) = url.get("url").and_then(|u| u.as_str()) {
                    if inner.starts_with("data:image") {
                        return true;
                    }
                }
            }
            map.values().any(walk_find_image)
        }
        Value::Array(arr) => arr.iter().any(walk_find_image),
        _ => false,
    }
}

fn optimize_input_array(input: &mut Value, stats: &mut OptimizeStats, aggressive: bool) {
    let Some(items) = input.as_array_mut() else {
        return;
    };
    let n = items.len();
    // Collect image locations (item_idx, part_idx) from oldest to newest.
    let mut image_locs: Vec<(usize, usize, String)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
            for (pi, part) in content.iter().enumerate() {
                if let Some(url) = extract_image_url(part) {
                    if url.len() >= MIN_DATA_URL_CHARS || url.starts_with("data:") {
                        image_locs.push((i, pi, url));
                    }
                }
            }
        }
    }

    // Dedupe identical data URLs (keep first occurrence).
    let mut seen: HashMap<String, (usize, usize)> = HashMap::new();
    let mut dedupe_targets: Vec<(usize, usize, String)> = Vec::new();
    for (i, pi, url) in &image_locs {
        if !url.starts_with("data:") {
            continue;
        }
        let key = short_hash(url);
        if let Some(&(fi, fpi)) = seen.get(&key) {
            if (fi, fpi) != (*i, *pi) {
                dedupe_targets.push((*i, *pi, key.clone()));
            }
        } else {
            seen.insert(key, (*i, *pi));
        }
    }
    for (i, pi, key) in dedupe_targets {
        if let Some(content) = items[i].get_mut("content").and_then(|c| c.as_array_mut()) {
            if pi < content.len() {
                content[pi] = json!({
                    "type": "input_text",
                    "text": format!("[image already provided earlier in this request; ref={key}]")
                });
                stats.images_deduped += 1;
                stats.modified = true;
            }
        }
    }

    // Re-scan after dedupe for collapse budget (keep last MAX_FULL_IMAGES full).
    let mut remaining: Vec<(usize, usize)> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
            for (pi, part) in content.iter().enumerate() {
                if extract_image_url(part).is_some() {
                    remaining.push((i, pi));
                }
            }
        }
    }
    let max_full = if aggressive {
        MAX_FULL_IMAGES / 2
    } else {
        MAX_FULL_IMAGES
    }
    .max(2);
    if remaining.len() > max_full {
        let collapse_count = remaining.len() - max_full;
        let to_collapse: Vec<(usize, usize)> = remaining.iter().take(collapse_count).copied().collect();
        for (i, pi) in to_collapse {
            if let Some(content) = items[i].get_mut("content").and_then(|c| c.as_array_mut()) {
                if pi < content.len() {
                    let url = extract_image_url(&content[pi]).unwrap_or_default();
                    let key = short_hash(&url);
                    content[pi] = json!({
                        "type": "input_text",
                        "text": format!(
                            "[historical image collapsed to save tokens; ref={key}. Re-attach if still needed.]"
                        )
                    });
                    stats.images_collapsed += 1;
                    stats.modified = true;
                }
            }
        }
    }

    // Historical text / tool output truncation.
    let recent_start = n.saturating_sub(if aggressive { 6 } else { 12 });
    for (i, item) in items.iter_mut().enumerate() {
        let historical = i < recent_start;
        let limit = if aggressive {
            HISTORICAL_TEXT_SOFT / 2
        } else if historical {
            HISTORICAL_TEXT_SOFT
        } else {
            // Recent: only truncate extreme blobs (offload handles better).
            OFFLOAD_TEXT_MIN * 2
        };

        let ty = item.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
        if matches!(
            ty.as_str(),
            "function_call_output" | "custom_tool_call_output"
        ) {
            if let Some(obj) = item.as_object_mut() {
                if let Some(s) = obj.get("output").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                    // Never leave giant base64 in tool outputs.
                    if s.starts_with("data:image") && s.len() > MIN_DATA_URL_CHARS {
                        let key = short_hash(&s);
                        obj.insert(
                            "output".into(),
                            Value::String(format!(
                                "[image data URL removed from tool output; ref={key}]"
                            )),
                        );
                        stats.images_collapsed += 1;
                        stats.modified = true;
                    } else if s.len() > limit {
                        obj.insert(
                            "output".into(),
                            Value::String(truncate_text(&s, TRUNC_HEAD, TRUNC_TAIL)),
                        );
                        stats.texts_truncated += 1;
                        stats.modified = true;
                    }
                }
            }
        }

        if let Some(content) = item.get_mut("content").and_then(|c| c.as_array_mut()) {
            for part in content.iter_mut() {
                let pty = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if matches!(pty, "input_text" | "text" | "output_text") {
                    if let Some(obj) = part.as_object_mut() {
                        if let Some(s) =
                            obj.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
                        {
                            if s.len() > limit {
                                obj.insert(
                                    "text".into(),
                                    Value::String(truncate_text(&s, TRUNC_HEAD, TRUNC_TAIL)),
                                );
                                stats.texts_truncated += 1;
                                stats.modified = true;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn optimize_chat_messages(messages: &mut Value, stats: &mut OptimizeStats, aggressive: bool) {
    let Some(items) = messages.as_array_mut() else {
        return;
    };
    let n = items.len();
    let recent_start = n.saturating_sub(if aggressive { 6 } else { 12 });

    // Dedupe data URLs across chat content parts.
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut image_count = 0usize;
    // Count images first.
    for item in items.iter() {
        if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
            for part in content {
                if chat_image_url(part).is_some() {
                    image_count += 1;
                }
            }
        }
    }
    let max_full = if aggressive {
        MAX_FULL_IMAGES / 2
    } else {
        MAX_FULL_IMAGES
    }
    .max(2);
    let keep_from = image_count.saturating_sub(max_full);
    let mut seen_images = 0usize;

    for (i, item) in items.iter_mut().enumerate() {
        let historical = i < recent_start;
        let limit = if aggressive {
            HISTORICAL_TEXT_SOFT / 2
        } else if historical {
            HISTORICAL_TEXT_SOFT
        } else {
            OFFLOAD_TEXT_MIN * 2
        };

        if let Some(content) = item.get_mut("content").and_then(|c| c.as_array_mut()) {
            for part in content.iter_mut() {
                if let Some(url) = chat_image_url(part) {
                    let key = short_hash(&url);
                    if url.starts_with("data:") {
                        if seen.contains_key(&key) {
                            *part = json!({
                                "type": "text",
                                "text": format!("[image already provided earlier; ref={key}]")
                            });
                            stats.images_deduped += 1;
                            stats.modified = true;
                            continue;
                        }
                        seen.insert(key.clone(), ());
                    }
                    if seen_images < keep_from {
                        *part = json!({
                            "type": "text",
                            "text": format!("[historical image collapsed; ref={key}]")
                        });
                        stats.images_collapsed += 1;
                        stats.modified = true;
                        seen_images += 1;
                        continue;
                    }
                    seen_images += 1;
                }

                let pty = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if pty == "text" {
                    if let Some(obj) = part.as_object_mut() {
                        if let Some(s) =
                            obj.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
                        {
                            if s.len() > limit {
                                obj.insert(
                                    "text".into(),
                                    Value::String(truncate_text(&s, TRUNC_HEAD, TRUNC_TAIL)),
                                );
                                stats.texts_truncated += 1;
                                stats.modified = true;
                            }
                        }
                    }
                }
            }
        } else if let Some(s) = item
            .get("content")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
        {
            if s.len() > limit {
                if let Some(obj) = item.as_object_mut() {
                    obj.insert(
                        "content".into(),
                        Value::String(truncate_text(&s, TRUNC_HEAD, TRUNC_TAIL)),
                    );
                    stats.texts_truncated += 1;
                    stats.modified = true;
                }
            }
        }
    }
}

fn hard_strip_old_media(input: &mut Value, stats: &mut OptimizeStats) {
    let Some(items) = input.as_array_mut() else {
        return;
    };
    let n = items.len();
    let keep_tail = n.saturating_sub(4);
    for (i, item) in items.iter_mut().enumerate() {
        if i >= keep_tail {
            continue;
        }
        if let Some(content) = item.get_mut("content").and_then(|c| c.as_array_mut()) {
            for part in content.iter_mut() {
                if extract_image_url(part).is_some() {
                    *part = json!({
                        "type": "input_text",
                        "text": "[image stripped: request body over hard budget]"
                    });
                    stats.images_collapsed += 1;
                    stats.modified = true;
                }
            }
        }
    }
}

fn extract_image_url(part: &Value) -> Option<String> {
    let ty = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if ty == "input_image" || ty == "output_image" {
        if let Some(s) = part.get("image_url").and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
        if let Some(s) = part
            .get("image_url")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
        {
            return Some(s.to_string());
        }
    }
    None
}

fn chat_image_url(part: &Value) -> Option<String> {
    let ty = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if ty == "image_url" {
        if let Some(s) = part.get("image_url").and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
        if let Some(s) = part
            .get("image_url")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
        {
            return Some(s.to_string());
        }
    }
    extract_image_url(part)
}

fn short_hash(s: &str) -> String {
    let mut hasher = Sha256::new();
    // Hash a prefix + length to avoid hashing multi-MB strings fully in hot path
    // while still distinguishing large payloads.
    let bytes = s.as_bytes();
    let head_len = bytes.len().min(512);
    hasher.update(&bytes[..head_len]);
    hasher.update(s.len().to_le_bytes());
    if bytes.len() > 1024 {
        let start = bytes.len().saturating_sub(256);
        hasher.update(&bytes[start..]);
    }
    let hex = format!("{:x}", hasher.finalize());
    hex.chars().take(12).collect()
}

fn truncate_text(s: &str, head: usize, tail: usize) -> String {
    if s.len() <= head + tail + 64 {
        return s.to_string();
    }
    let head_s: String = s.chars().take(head).collect();
    let tail_s: String = s
        .chars()
        .rev()
        .take(tail)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!(
        "{head_s}\n\n…[truncated {} chars of {} total to reduce multi-turn token usage]…\n\n{tail_s}",
        s.len().saturating_sub(head + tail),
        s.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn big_data_url(tag: &str, n: usize) -> String {
        format!("data:image/png;base64,{}{}", tag, "A".repeat(n))
    }

    #[test]
    fn dedupes_identical_images() {
        let url = big_data_url("same", 3000);
        let mut body = json!({
            "model": "grok-4.5",
            "input": [
                {"role":"user","content":[
                    {"type":"input_image","image_url": url},
                    {"type":"input_text","text":"first"}
                ]},
                {"role":"user","content":[
                    {"type":"input_image","image_url": url},
                    {"type":"input_text","text":"second"}
                ]}
            ]
        });
        let stats = optimize_responses_payload(&mut body);
        assert!(stats.images_deduped >= 1);
        assert_eq!(body["store"], json!(false));
        let second = &body["input"][1]["content"][0];
        assert_eq!(second["type"], "input_text");
    }

    #[test]
    fn collapses_excess_images() {
        let mut parts = Vec::new();
        for i in 0..12 {
            parts.push(json!({
                "role": "user",
                "content": [
                    {"type":"input_image","image_url": big_data_url(&format!("img{i}"), 2500)},
                    {"type":"input_text","text": format!("see image {i}")}
                ]
            }));
        }
        let mut body = json!({"model":"grok-4.5","input": parts});
        let stats = optimize_responses_payload(&mut body);
        assert!(stats.images_collapsed >= 1);
        let remaining_images = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|it| it["content"].as_array().unwrap())
            .filter(|p| p["type"] == "input_image")
            .count();
        assert!(remaining_images <= MAX_FULL_IMAGES);
    }

    #[test]
    fn truncates_historical_tool_outputs() {
        let big = "X".repeat(40_000);
        let mut input = Vec::new();
        for i in 0..15 {
            input.push(json!({
                "type": "function_call_output",
                "call_id": format!("c{i}"),
                "output": big
            }));
        }
        input.push(json!({
            "role": "user",
            "content": [{"type":"input_text","text":"continue"}]
        }));
        let mut body = json!({"model":"grok-4.5","input": input});
        let stats = optimize_responses_payload(&mut body);
        assert!(stats.texts_truncated >= 1);
        let first_out = body["input"][0]["output"].as_str().unwrap();
        assert!(first_out.len() < 40_000);
        assert!(first_out.contains("truncated"));
    }

    #[test]
    fn forces_store_false_with_images() {
        let mut body = json!({
            "model": "grok-4.5",
            "store": true,
            "input": [{
                "role":"user",
                "content":[
                    {"type":"input_image","image_url": big_data_url("x", 100)},
                    {"type":"input_text","text":"hi"}
                ]
            }]
        });
        let stats = optimize_responses_payload(&mut body);
        assert!(stats.store_forced_false);
        assert_eq!(body["store"], false);
    }
}
