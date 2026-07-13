//! Intercept Codex/OpenAI `image_gen` / `image_generation` tool calls and fulfill
//! them with xAI Grok Imagine, so Codex sees "built-in" image generation behavior
//! when using grok-go as the Responses backend.

use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use crate::auth::ensure_fresh_token;
use crate::config::{load_auth, AppConfig};
use crate::error::AppResult;
use crate::gateway::media_artifacts::{materialize_image_response, materialize_image_response_sync};
use crate::paths::artifacts_dir;
use crate::router::{pick_account_for, replace_account_tokens, MediaCapability};

/// Names we treat as Codex / OpenAI image generation tools.
pub fn is_image_gen_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "image_gen" | "image_generation" | "imagegen" | "generate_image" | "generateimage"
    )
}

pub fn image_gen_function_tool() -> Value {
    json!({
        "type": "function",
        "name": "image_gen",
        "description": "Generate an image with Grok Imagine (Codex image_gen compatible). Call immediately for draw/generate/create image requests. Do not search the repo. Returns absolute local filesystem paths in path/files plus markdown ![image](/abs/path) for direct Codex rendering — never remote CDN URLs.",
        "parameters": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Detailed image generation or edit prompt. Required."
                },
                "size": {
                    "type": "string",
                    "description": "Optional size, e.g. 1024x1024"
                },
                "quality": {
                    "type": "string",
                    "enum": ["low", "medium", "high"],
                    "description": "Optional quality hint: low | medium | high"
                },
                "n": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 4,
                    "description": "Number of images (default 1, max 4)"
                },
                "image_url": {
                    "type": "string",
                    "description": "Optional reference image URL for edits (https:// or data URL)"
                }
            },
            "required": ["prompt"]
        }
    })
}

pub fn collect_image_gen_calls(response: &Value) -> Vec<Value> {
    let mut calls = Vec::new();
    if let Some(output) = response.get("output").and_then(|o| o.as_array()) {
        for item in output {
            let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if (ty == "function_call" || ty == "custom_tool_call") && is_image_gen_name(name) {
                calls.push(item.clone());
            }
            if ty == "image_generation_call" && item.get("status").and_then(|s| s.as_str()) == Some("in_progress") {
                calls.push(item.clone());
            }
        }
    }
    calls
}

/// Run Grok Imagine for one image_gen function_call / custom_tool_call item.
///
/// Prefer `sticky_token` from the parent `/responses` request so multi-step tool
/// loops stay on the same OAuth account (avoids WRR flip mid-turn).
pub async fn fulfill_image_gen_call(
    client: &reqwest::Client,
    config: &AppConfig,
    call: &Value,
    sticky_token: Option<&str>,
) -> AppResult<Value> {
    let call_id = call
        .get("call_id")
        .or_else(|| call.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("image_gen_call")
        .to_string();

    let args = parse_call_args(call);
    let prompt = args
        .get("prompt")
        .or_else(|| args.get("input"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if prompt.trim().is_empty() {
        return Ok(function_call_output(
            &call_id,
            json!({"error": "missing prompt"}).to_string(),
        ));
    }

    let image_url = args
        .get("image_url")
        .or_else(|| args.get("image"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let n = args.get("n").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 4) as usize;
    let model = config.default_image_model.clone();

    let token = if let Some(t) = sticky_token.map(str::trim).filter(|s| !s.is_empty()) {
        t.to_string()
    } else {
        let store = load_auth()?;
        let mut account = pick_account_for(config, &store, MediaCapability::Image)?;
        let before = account.access_token.clone();
        let t = ensure_fresh_token(config, &mut account).await?;
        if account.access_token != before {
            replace_account_tokens(&account)?;
        }
        t
    };

    let (path, body) = if let Some(ref url) = image_url {
        (
            "/images/edits",
            json!({
                "model": model,
                "prompt": prompt,
                "image": {"url": url, "type": "image_url"},
                "n": n
            }),
        )
    } else {
        (
            "/images/generations",
            json!({
                "model": model,
                "prompt": prompt,
                "n": n
            }),
        )
    };

    let url = format!("{}{}", config.xai_base_url.trim_end_matches('/'), path);
    let resp = client
        .post(&url)
        .bearer_auth(&token)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let value: Value = resp.json().await.unwrap_or(json!({}));
    if !status.is_success() {
        return Ok(function_call_output(
            &call_id,
            json!({
                "error": "image generation failed",
                "status": status.as_u16(),
                "body": value
            })
            .to_string(),
        ));
    }

    // Always land remote CDN / b64 into absolute local paths for Codex rendering.
    let saved = materialize_image_response(client, &value)
        .await
        .unwrap_or_else(|err| {
            tracing::warn!("materialize_image_response failed: {err}");
            materialize_image_response_sync(&value).unwrap_or_default()
        });
    let b64_list = extract_b64_list(&value);
    let primary = saved.first().cloned();

    // Codex built-in style item (also keep function_call_output for tool loops).
    let output_payload = json!({
        "ok": true,
        "model": model,
        "prompt": prompt,
        "path": primary,
        "file": primary,
        "files": saved,
        "count": b64_list.len().max(saved.len()),
        "markdown": primary.as_ref().map(|p| format!("![image]({p})")),
        // Keep a short b64 note only for bridge completeness; prefer files/path for display.
        "b64_json": b64_list.first().cloned(),
        "display_hint": "Render with absolute local path from path/files (Markdown ![image](/abs/path)). Do not use remote CDN urls.",
        "note": "Generated by Grok Imagine via grok-go image_gen bridge; files are local absolute paths"
    });

    Ok(function_call_output(&call_id, output_payload.to_string()))
}

/// Convert fulfilled image outputs into Codex-friendly `image_generation_call` items
/// appended to the final response output (alongside any remaining messages).
pub fn inject_image_generation_calls(response: &mut Value, fulfilled: &[(Value, Value)]) {
    let Some(output) = response.get_mut("output").and_then(|o| o.as_array_mut()) else {
        return;
    };
    for (call, result) in fulfilled {
        let call_id = call
            .get("call_id")
            .or_else(|| call.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("img_call");
        let result_text = result
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let parsed: Value = serde_json::from_str(result_text).unwrap_or(json!({}));
        let b64 = parsed
            .get("b64_json")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let path = parsed
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.pointer("/files/0").and_then(|v| v.as_str()))
            .unwrap_or("");

        output.push(json!({
            "type": "image_generation_call",
            "id": format!("ig_{call_id}"),
            "status": "completed",
            // Prefer empty result when path is available so clients render local files
            // instead of huge/ephemeral remote or b64 payloads.
            "result": if path.is_empty() { b64 } else { "" },
            "output": result_text,
            "path": path,
            "files": parsed.get("files").cloned().unwrap_or(json!([])),
            "markdown": parsed.get("markdown").cloned().unwrap_or(json!(null)),
            "provider": "grok-imagine",
        }));
    }
}

fn function_call_output(call_id: &str, output: String) -> Value {
    json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": output
    })
}

fn parse_call_args(call: &Value) -> Value {
    if let Some(args) = call.get("arguments") {
        match args {
            Value::String(s) => serde_json::from_str(s).unwrap_or(json!({"prompt": s})),
            Value::Object(_) => args.clone(),
            _ => json!({}),
        }
    } else if let Some(input) = call.get("input") {
        match input {
            Value::String(s) => json!({"prompt": s}),
            other => other.clone(),
        }
    } else {
        json!({})
    }
}

fn extract_b64_list(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(data) = value.get("data").and_then(|d| d.as_array()) {
        for item in data {
            if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
                out.push(b64.to_string());
            }
        }
    }
    out
}

/// Max server-side image tool loop iterations.
pub const MAX_IMAGE_TOOL_ROUNDS: usize = 3;

/// Whether any tools in the request are image-generation related.
pub fn request_has_image_tools(tools: &Value) -> bool {
    let Some(arr) = tools.as_array() else {
        return false;
    };
    arr.iter().any(|t| {
        let ty = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
        matches!(ty, "image_generation" | "image_gen")
            || is_image_gen_name(name)
            || (ty == "function" && is_image_gen_name(name))
            || (ty == "custom" && is_image_gen_name(name))
    })
}

pub fn track_image_gen_tools(tools: &Value, set: &mut HashSet<String>) {
    let Some(arr) = tools.as_array() else {
        return;
    };
    for t in arr {
        let ty = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(ty, "image_generation" | "image_gen") || is_image_gen_name(name) {
            set.insert("image_gen".into());
        }
        if ty == "function" && is_image_gen_name(name) {
            set.insert(name.to_string());
        }
        if ty == "custom" && is_image_gen_name(name) {
            set.insert(name.to_string());
        }
    }
}

#[allow(dead_code)]
pub fn artifacts_path() -> AppResult<PathBuf> {
    artifacts_dir()
}
