//! Persist image/video results under `~/.grok-go/artifacts/` and return
//! absolute local filesystem paths that Codex can render directly.
//!
//! Remote CDN URLs (imgen.x.ai / vidgen.x.ai) are temporary and often fail to
//! display inside Codex; local absolute paths work with Markdown image syntax.

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::paths::artifacts_dir;

/// Absolute path as a plain string (no `file://` prefix) for Codex Markdown.
pub fn abs_path_string(path: &Path) -> String {
    if path.is_absolute() {
        path.display().to_string()
    } else {
        // Best-effort absolutize relative paths.
        std::env::current_dir()
            .map(|cwd| cwd.join(path).display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }
}

fn mime_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// Normalize a media input for upstream xAI APIs.
/// Accepts: `https://...`, `data:...`, absolute local path, or `file://...`.
/// Local files are inlined as `data:<mime>;base64,...` so agents can pass
/// Codex/local artifact paths without hunting for a public URL.
pub fn resolve_media_url(input: &str) -> AppResult<String> {
    let s = input.trim();
    if s.is_empty() {
        return Err(AppError::msg("empty media url/path"));
    }
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") {
        return Ok(s.to_string());
    }

    let path = if let Some(stripped) = s.strip_prefix("file://") {
        Path::new(stripped).to_path_buf()
    } else {
        Path::new(s).to_path_buf()
    };
    if !path.exists() {
        return Err(AppError::msg(format!(
            "media path does not exist: {}",
            path.display()
        )));
    }
    let bytes = fs::read(&path)
        .map_err(|e| AppError::msg(format!("read media path {}: {e}", path.display())))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let mime = mime_from_path(&path);
    Ok(format!("data:{mime};base64,{b64}"))
}

fn stamp() -> String {
    chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f").to_string()
}

fn guess_ext_from_url(url: &str, default: &str) -> String {
    let clean = url.split('?').next().unwrap_or(url);
    let lower = clean.to_ascii_lowercase();
    for (needle, ext) in [
        (".png", "png"),
        (".jpg", "jpg"),
        (".jpeg", "jpeg"),
        (".webp", "webp"),
        (".gif", "gif"),
        (".mp4", "mp4"),
        (".webm", "webm"),
        (".mov", "mov"),
    ] {
        if lower.ends_with(needle) {
            return ext.to_string();
        }
    }
    default.to_string()
}

fn guess_ext_from_bytes(bytes: &[u8], default: &str) -> String {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return "png".into();
    }
    if bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff {
        return "jpg".into();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "webp".into();
    }
    if bytes.len() >= 6 && (&bytes[0..6] == b"GIF87a" || &bytes[0..6] == b"GIF89a") {
        return "gif".into();
    }
    // ISO BMFF / MP4: ....ftyp
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return "mp4".into();
    }
    default.to_string()
}

fn base64_decode(s: &str) -> AppResult<Vec<u8>> {
    use base64::Engine;
    let trimmed = s.trim();
    // Support data URLs: data:image/png;base64,....
    let payload = if let Some(idx) = trimmed.find("base64,") {
        &trimmed[idx + "base64,".len()..]
    } else {
        trimmed
    };
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(payload))
        .map_err(|e| AppError::msg(format!("base64 decode failed: {e}")))
}

/// Write raw bytes into artifacts dir; returns absolute path string.
pub fn write_bytes(prefix: &str, bytes: &[u8], preferred_ext: &str) -> AppResult<String> {
    let dir = artifacts_dir()?;
    fs::create_dir_all(&dir)?;
    let ext = guess_ext_from_bytes(bytes, preferred_ext);
    let name = format!("{prefix}-{}.{}", stamp(), ext);
    let path = dir.join(name);
    fs::write(&path, bytes)?;
    Ok(abs_path_string(&path))
}

/// Download a remote URL (or data: URL) into artifacts; returns absolute path.
pub async fn download_url_to_artifacts(
    client: &reqwest::Client,
    url: &str,
    prefix: &str,
    default_ext: &str,
) -> AppResult<String> {
    let url = url.trim();
    if url.is_empty() {
        return Err(AppError::msg("empty media url"));
    }

    // Already a local path — keep as absolute.
    if url.starts_with('/') && Path::new(url).exists() {
        return Ok(abs_path_string(Path::new(url)));
    }
    if let Some(stripped) = url.strip_prefix("file://") {
        let path = Path::new(stripped);
        if path.exists() {
            return Ok(abs_path_string(path));
        }
    }

    if url.starts_with("data:") {
        let bytes = base64_decode(url)?;
        return write_bytes(prefix, &bytes, default_ext);
    }

    let resp = client
        .get(url)
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| AppError::msg(format!("download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::msg(format!(
            "download HTTP {}: {}",
            resp.status(),
            url
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::msg(format!("download body failed: {e}")))?;
    let ext = guess_ext_from_bytes(&bytes, &guess_ext_from_url(url, default_ext));
    write_bytes(prefix, &bytes, &ext)
}

fn extract_image_urls(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(data) = value.get("data").and_then(|d| d.as_array()) {
        for item in data {
            if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                if !url.trim().is_empty() {
                    out.push(url.to_string());
                }
            }
        }
    }
    // Some payloads nest under result/image
    if let Some(url) = value.pointer("/image/url").and_then(|v| v.as_str()) {
        out.push(url.to_string());
    }
    out
}

fn extract_b64_list(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(data) = value.get("data").and_then(|d| d.as_array()) {
        for item in data {
            if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
                if !b64.trim().is_empty() {
                    out.push(b64.to_string());
                }
            }
        }
    }
    out
}

/// Materialize image generation/edit JSON into local files.
/// Returns only absolute local paths (never remote URLs).
pub async fn materialize_image_response(
    client: &reqwest::Client,
    value: &Value,
) -> AppResult<Vec<String>> {
    let mut paths = Vec::new();

    for (i, b64) in extract_b64_list(value).into_iter().enumerate() {
        match base64_decode(&b64) {
            Ok(raw) => match write_bytes(&format!("img-{i}"), &raw, "png") {
                Ok(p) => paths.push(p),
                Err(err) => tracing::warn!("failed to write b64 image: {err}"),
            },
            Err(err) => tracing::warn!("invalid b64 image: {err}"),
        }
    }

    // Prefer downloading URLs only when we don't already have local files from b64
    // for the same slot. Always download URL items that have no b64.
    if let Some(data) = value.get("data").and_then(|d| d.as_array()) {
        for (i, item) in data.iter().enumerate() {
            let has_b64 = item
                .get("b64_json")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if has_b64 {
                continue;
            }
            if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                match download_url_to_artifacts(client, url, &format!("img-{i}"), "jpg").await {
                    Ok(p) => paths.push(p),
                    Err(err) => tracing::warn!("image url download failed: {err}"),
                }
            }
        }
    } else {
        for (i, url) in extract_image_urls(value).into_iter().enumerate() {
            match download_url_to_artifacts(client, &url, &format!("img-{i}"), "jpg").await {
                Ok(p) => paths.push(p),
                Err(err) => tracing::warn!("image url download failed: {err}"),
            }
        }
    }

    Ok(paths)
}

/// Sync variant used by image_bridge when only b64 is available (no download).
/// For URL-only results, returns empty — prefer async materialize.
pub fn materialize_image_response_sync(value: &Value) -> AppResult<Vec<String>> {
    let mut paths = Vec::new();
    for (i, b64) in extract_b64_list(value).into_iter().enumerate() {
        let raw = base64_decode(&b64)?;
        paths.push(write_bytes(&format!("img-{i}"), &raw, "png")?);
    }
    Ok(paths)
}

fn extract_video_url(value: &Value) -> Option<String> {
    value
        .pointer("/video/url")
        .or_else(|| value.pointer("/data/0/url"))
        .or_else(|| value.get("url"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

/// Download video result JSON to a local mp4 path.
pub async fn materialize_video_response(
    client: &reqwest::Client,
    value: &Value,
) -> AppResult<Vec<String>> {
    let mut paths = Vec::new();
    if let Some(url) = extract_video_url(value) {
        let path = download_url_to_artifacts(client, &url, "vid", "mp4").await?;
        paths.push(path);
    }
    Ok(paths)
}

/// Poll xAI deferred video job until done/failed or timeout.
pub async fn poll_video_result(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    request_id: &str,
    timeout: Duration,
) -> AppResult<Value> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/videos/{request_id}");
    let started = std::time::Instant::now();
    let mut delay = Duration::from_millis(1500);

    loop {
        let resp = client
            .get(&url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| AppError::msg(format!("video poll failed: {e}")))?;
        let status = resp.status();
        let value: Value = resp.json().await.unwrap_or(json!({}));
        if !status.is_success() {
            // xAI returns 404 "Failed to read static file" when the job was submitted
            // under a different OAuth account (or the id is unknown).
            let hint = if status.as_u16() == 404 {
                " (hint: video jobs are account-scoped — submit and poll must use the same OAuth token)"
            } else {
                ""
            };
            return Err(AppError::msg(format!(
                "video poll HTTP {status}: {value}{hint}"
            )));
        }

        let job_status = value
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match job_status {
            "done" | "completed" | "succeeded" => return Ok(value),
            "failed" | "error" => {
                return Err(AppError::msg(format!("video generation failed: {value}")));
            }
            _ => {
                if started.elapsed() >= timeout {
                    return Err(AppError::msg(format!(
                        "video poll timed out after {:?} (last status={job_status})",
                        timeout
                    )));
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
}

/// Build a Codex-friendly media summary. `files` are absolute local paths only.
pub fn media_summary(
    tool: &str,
    model: &str,
    prompt: &str,
    files: &[String],
    upstream: &Value,
    kind: &str, // "image" | "video"
) -> Value {
    let primary = files.first().cloned();
    let markdown = primary.as_ref().map(|p| {
        if kind == "video" {
            // Codex desktop can show local videos via absolute path in Markdown image-like syntax.
            format!("![video]({p})")
        } else {
            format!("![image]({p})")
        }
    });

    json!({
        "ok": true,
        "tool": tool,
        "model": model,
        "prompt": prompt,
        "kind": kind,
        // Primary absolute path for quick access
        "path": primary,
        "file": primary,
        // All local absolute paths
        "files": files,
        // R2-02: unified artifacts[] for agents (same as files)
        "artifacts": files,
        "summary": primary
            .as_ref()
            .map(|p| format!("{kind} ready: {p}"))
            .unwrap_or_else(|| format!("{kind} completed")),
        "result": {
            "path": primary,
            "files": files,
            "markdown": markdown,
        },
        "error": null,
        // Ready-to-paste Markdown for Codex desktop rendering
        "markdown": markdown,
        "display_hint": "Use the absolute path from `path`/`files` (or `markdown`) to render inline. Do NOT use remote CDN urls.",
        // Compact upstream for light debug (not full b64)
        "upstream": compact_upstream(upstream),
    })
}

fn compact_upstream(value: &Value) -> Value {
    let mut v = value.clone();
    // Drop heavy b64 payloads from the tool text response.
    if let Some(data) = v.get_mut("data").and_then(|d| d.as_array_mut()) {
        for item in data.iter_mut() {
            if let Some(obj) = item.as_object_mut() {
                if let Some(b64) = obj.get("b64_json").and_then(|x| x.as_str()) {
                    let len = b64.len();
                    obj.insert(
                        "b64_json".into(),
                        Value::String(format!("[omitted {len} chars]")),
                    );
                }
            }
        }
    }
    v
}

/// MCP tool result: text JSON + optional markdown line pointing at local path.
pub fn mcp_media_content(summary: &Value) -> Value {
    let mut content = vec![json!({
        "type": "text",
        "text": serde_json::to_string_pretty(summary).unwrap_or_else(|_| summary.to_string())
    })];
    if let Some(md) = summary.get("markdown").and_then(|v| v.as_str()) {
        content.push(json!({
            "type": "text",
            "text": format!(
                "Render in Codex with absolute local path:\n{md}\n\npath: {}",
                summary.get("path").and_then(|v| v.as_str()).unwrap_or("")
            )
        }));
    }
    json!({ "content": content })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn abs_path_keeps_absolute() {
        let p = PathBuf::from("/tmp/foo.png");
        assert_eq!(abs_path_string(&p), "/tmp/foo.png");
    }

    #[test]
    fn guess_png_magic() {
        let png = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0];
        assert_eq!(guess_ext_from_bytes(&png, "bin"), "png");
    }

    #[test]
    fn media_summary_image_markdown() {
        let files = vec!["/Users/me/.grok-go/artifacts/img.png".to_string()];
        let s = media_summary("image_gen", "m", "p", &files, &json!({}), "image");
        assert_eq!(s["path"], "/Users/me/.grok-go/artifacts/img.png");
        assert_eq!(
            s["markdown"],
            "![image](/Users/me/.grok-go/artifacts/img.png)"
        );
        assert!(s["files"].as_array().unwrap().len() == 1);
    }

    #[test]
    fn media_summary_video_markdown() {
        let files = vec!["/tmp/v.mp4".to_string()];
        let s = media_summary("video_generate", "m", "p", &files, &json!({}), "video");
        assert_eq!(s["markdown"], "![video](/tmp/v.mp4)");
    }

    #[test]
    fn compact_upstream_strips_b64() {
        let up = json!({"data": [{"b64_json": "AAAABBBB", "url": "https://x"}]});
        let c = compact_upstream(&up);
        let b64 = c["data"][0]["b64_json"].as_str().unwrap();
        assert!(b64.starts_with("[omitted"));
        assert_eq!(c["data"][0]["url"], "https://x");
    }

    #[test]
    fn materialize_sync_writes_local_png() {
        // 1x1 PNG
        let b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let value = json!({"data": [{"b64_json": b64}]});
        let paths = materialize_image_response_sync(&value).expect("write");
        assert_eq!(paths.len(), 1);
        let p = std::path::Path::new(&paths[0]);
        assert!(p.is_absolute(), "expected absolute path, got {}", paths[0]);
        assert!(p.exists());
        assert!(paths[0].ends_with(".png") || paths[0].contains(".png"));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn mcp_media_content_includes_markdown_block() {
        let summary = media_summary(
            "image_gen",
            "m",
            "p",
            &["/tmp/x.png".into()],
            &json!({}),
            "image",
        );
        let out = mcp_media_content(&summary);
        let content = out["content"].as_array().unwrap();
        assert!(content.len() >= 2);
        let text = content[1]["text"].as_str().unwrap();
        assert!(text.contains("![image](/tmp/x.png)"));
        assert!(text.contains("path: /tmp/x.png"));
    }

    #[test]
    fn resolve_media_url_passthrough_https() {
        let u = resolve_media_url("https://example.com/a.png").unwrap();
        assert_eq!(u, "https://example.com/a.png");
    }

    #[test]
    fn resolve_media_url_local_file_to_data() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("grok-go-media-test-{}.png", stamp()));
        // minimal PNG header is enough for the path→data conversion
        std::fs::write(&path, [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']).unwrap();
        let data = resolve_media_url(path.to_str().unwrap()).unwrap();
        assert!(data.starts_with("data:image/png;base64,"));
        let _ = std::fs::remove_file(&path);
    }
}
