//! OpenAI Chat Completions SSE → Anthropic Messages SSE.
//!
//! Event order Claude Code expects:
//! 1. `message_start`
//! 2. for each block: `content_block_start` → `content_block_delta`* → `content_block_stop`
//! 3. `message_delta` (stop_reason + usage)
//! 4. `message_stop`

use super::response::{map_stop_reason, ThinkingMode};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
enum BlockKind {
    Text,
    ToolUse,
    Thinking,
}

#[derive(Debug, Default)]
pub struct OpenAiToAnthropicSse {
    buffer: String,
    message_id: Option<String>,
    model: Option<String>,
    has_message_start: bool,
    /// Anthropic content block index (sequential across text/tools).
    next_index: usize,
    current: Option<(usize, BlockKind)>,
    /// OpenAI tool_calls[].index → Anthropic block index
    tool_index_map: HashMap<usize, usize>,
    /// OpenAI tool index → tool id (for late name/args chunks)
    tool_ids: HashMap<usize, String>,
    /// True after `message_delta` (finish_reason) was emitted.
    saw_message_delta: bool,
    finished: bool,
    /// Latest usage from upstream (may arrive on a trailing usage-only chunk).
    last_input_tokens: u64,
    last_output_tokens: u64,
    thinking_mode: ThinkingMode,
}

impl OpenAiToAnthropicSse {
    pub fn new() -> Self {
        Self {
            thinking_mode: ThinkingMode::Hide,
            ..Self::default()
        }
    }

    pub fn with_thinking_mode(mode: ThinkingMode) -> Self {
        Self {
            thinking_mode: mode,
            ..Self::default()
        }
    }

    /// Ingest an upstream SSE byte chunk; returns Anthropic SSE bytes to forward.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        if self.finished {
            return Vec::new();
        }
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut out = Vec::new();

        while let Some(pos) = self.buffer.find("\n\n") {
            let frame = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 2..].to_string();
            if let Some(bytes) = self.handle_frame(&frame) {
                out.extend_from_slice(&bytes);
            }
        }
        out
    }

    /// Flush any remainder (best-effort) and ensure stream ends with message_stop.
    pub fn finish(&mut self) -> Vec<u8> {
        if self.finished {
            return Vec::new();
        }
        let mut out = Vec::new();
        if !self.buffer.trim().is_empty() {
            let frame = std::mem::take(&mut self.buffer);
            if let Some(bytes) = self.handle_frame(&frame) {
                out.extend_from_slice(&bytes);
            }
        }
        if !self.finished {
            out.extend_from_slice(&self.close_current_block());
            if self.has_message_start {
                // Only synthesize message_delta if upstream never sent finish_reason.
                if !self.saw_message_delta {
                    out.extend_from_slice(&encode_event(
                        "message_delta",
                        &json!({
                            "type": "message_delta",
                            "delta": {
                                "stop_reason": "end_turn",
                                "stop_sequence": null
                            },
                            "usage": {
                                "input_tokens": self.last_input_tokens,
                                "output_tokens": self.last_output_tokens
                            }
                        }),
                    ));
                    self.saw_message_delta = true;
                }
                out.extend_from_slice(&encode_event(
                    "message_stop",
                    &json!({"type": "message_stop"}),
                ));
            }
            self.finished = true;
        }
        out
    }

    fn handle_frame(&mut self, frame: &str) -> Option<Vec<u8>> {
        let mut data_line: Option<&str> = None;
        for line in frame.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix("data:") {
                data_line = Some(rest.trim());
            }
        }
        let data = data_line?;
        if data.is_empty() {
            return None;
        }
        if data == "[DONE]" {
            let mut out = self.close_current_block();
            if self.has_message_start && !self.finished {
                if !self.saw_message_delta {
                    out.extend_from_slice(&encode_event(
                        "message_delta",
                        &json!({
                            "type": "message_delta",
                            "delta": {
                                "stop_reason": "end_turn",
                                "stop_sequence": null
                            },
                            "usage": {
                                "input_tokens": self.last_input_tokens,
                                "output_tokens": self.last_output_tokens
                            }
                        }),
                    ));
                    self.saw_message_delta = true;
                }
                out.extend_from_slice(&encode_event(
                    "message_stop",
                    &json!({"type": "message_stop"}),
                ));
                self.finished = true;
            }
            return Some(out);
        }

        let chunk: Value = serde_json::from_str(data).ok()?;
        Some(self.handle_chunk(&chunk))
    }

    fn handle_chunk(&mut self, chunk: &Value) -> Vec<u8> {
        let mut out = Vec::new();

        if self.message_id.is_none() {
            if let Some(id) = chunk.get("id").and_then(|i| i.as_str()) {
                self.message_id = Some(if id.starts_with("msg_") {
                    id.to_string()
                } else {
                    format!("msg_{id}")
                });
            }
        }
        if self.model.is_none() {
            if let Some(m) = chunk.get("model").and_then(|m| m.as_str()) {
                self.model = Some(m.to_string());
            }
        }

        if !self.has_message_start {
            out.extend_from_slice(&encode_event(
                "message_start",
                &json!({
                    "type": "message_start",
                    "message": {
                        "id": self.message_id.clone().unwrap_or_else(|| "msg_stream".into()),
                        "type": "message",
                        "role": "assistant",
                        "model": self.model.clone().unwrap_or_default(),
                        "content": [],
                        "stop_reason": null,
                        "stop_sequence": null,
                        "usage": {
                            "input_tokens": 0,
                            "output_tokens": 0
                        }
                    }
                }),
            ));
            self.has_message_start = true;
        }

        // Some providers put usage only on a final chunk with empty choices.
        let choices = chunk.get("choices").and_then(|c| c.as_array());
        let choice = choices.and_then(|c| c.first());

        if let Some(choice) = choice {
            let delta = choice.get("delta").cloned().unwrap_or(json!({}));

            // reasoning / thinking (skipped when ThinkingMode::Hide for TTFT)
            if self.thinking_mode != ThinkingMode::Hide {
                if let Some(reasoning) = delta
                    .get("reasoning_content")
                    .or_else(|| delta.get("reasoning"))
                    .and_then(|r| r.as_str())
                {
                    if !reasoning.is_empty() {
                        let text = match self.thinking_mode {
                            ThinkingMode::Summary => {
                                // Stream only first ~120 chars once per block
                                let t = reasoning.trim();
                                if t.chars().count() > 120 {
                                    format!("{}…", t.chars().take(120).collect::<String>())
                                } else {
                                    t.to_string()
                                }
                            }
                            _ => reasoning.to_string(),
                        };
                        out.extend_from_slice(&self.ensure_block(BlockKind::Thinking));
                        if let Some((index, BlockKind::Thinking)) = &self.current {
                            let index = *index;
                            out.extend_from_slice(&encode_event(
                                "content_block_delta",
                                &json!({
                                    "type": "content_block_delta",
                                    "index": index,
                                    "delta": {
                                        "type": "thinking_delta",
                                        "thinking": text
                                    }
                                }),
                            ));
                        }
                    }
                }
            }

            // text
            if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                if !text.is_empty() {
                    out.extend_from_slice(&self.ensure_block(BlockKind::Text));
                    if let Some((index, BlockKind::Text)) = &self.current {
                        let index = *index;
                        out.extend_from_slice(&encode_event(
                            "content_block_delta",
                            &json!({
                                "type": "content_block_delta",
                                "index": index,
                                "delta": {
                                    "type": "text_delta",
                                    "text": text
                                }
                            }),
                        ));
                    }
                }
            }

            // tool_calls
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let oi = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                        if !id.is_empty() {
                            self.tool_ids.insert(oi, id.to_string());
                        }
                    }
                    let name = tc
                        .pointer("/function/name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let args = tc
                        .pointer("/function/arguments")
                        .and_then(|a| a.as_str())
                        .unwrap_or("");

                    // Start block when we have a name (id may arrive same chunk or earlier).
                    if !name.is_empty() && !self.tool_index_map.contains_key(&oi) {
                        out.extend_from_slice(&self.close_current_block());
                        let index = self.next_index;
                        self.next_index += 1;
                        self.tool_index_map.insert(oi, index);
                        let id = self
                            .tool_ids
                            .get(&oi)
                            .cloned()
                            .unwrap_or_else(|| format!("call_{oi}"));
                        out.extend_from_slice(&encode_event(
                            "content_block_start",
                            &json!({
                                "type": "content_block_start",
                                "index": index,
                                "content_block": {
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": {}
                                }
                            }),
                        ));
                        self.current = Some((index, BlockKind::ToolUse));
                    }

                    if !args.is_empty() {
                        // Ensure block exists even if name came without id first.
                        if !self.tool_index_map.contains_key(&oi) {
                            // Arguments-only chunk: open placeholder if id known.
                            if let Some(id) = self.tool_ids.get(&oi).cloned() {
                                out.extend_from_slice(&self.close_current_block());
                                let index = self.next_index;
                                self.next_index += 1;
                                self.tool_index_map.insert(oi, index);
                                out.extend_from_slice(&encode_event(
                                    "content_block_start",
                                    &json!({
                                        "type": "content_block_start",
                                        "index": index,
                                        "content_block": {
                                            "type": "tool_use",
                                            "id": id,
                                            "name": "",
                                            "input": {}
                                        }
                                    }),
                                ));
                                self.current = Some((index, BlockKind::ToolUse));
                            }
                        }
                        if let Some(&index) = self.tool_index_map.get(&oi) {
                            // Switch current tracking to this tool block for close ordering.
                            if self.current.as_ref().map(|(i, _)| *i) != Some(index) {
                                out.extend_from_slice(&self.close_current_block());
                                self.current = Some((index, BlockKind::ToolUse));
                            }
                            out.extend_from_slice(&encode_event(
                                "content_block_delta",
                                &json!({
                                    "type": "content_block_delta",
                                    "index": index,
                                    "delta": {
                                        "type": "input_json_delta",
                                        "partial_json": args
                                    }
                                }),
                            ));
                        }
                    }
                }
            }

            if let Some(finish) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                let has_tools = !self.tool_index_map.is_empty();
                out.extend_from_slice(&self.close_current_block());
                let stop = map_stop_reason(Some(finish), has_tools);
                let usage = chunk.get("usage");
                let input_tokens = usage
                    .and_then(|u| u.get("prompt_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(self.last_input_tokens);
                let output_tokens = usage
                    .and_then(|u| u.get("completion_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(self.last_output_tokens);
                if input_tokens > 0 {
                    self.last_input_tokens = input_tokens;
                }
                if output_tokens > 0 {
                    self.last_output_tokens = output_tokens;
                }
                out.extend_from_slice(&encode_event(
                    "message_delta",
                    &json!({
                        "type": "message_delta",
                        "delta": {
                            "stop_reason": stop,
                            "stop_sequence": null
                        },
                        "usage": {
                            "input_tokens": self.last_input_tokens,
                            "output_tokens": self.last_output_tokens
                        }
                    }),
                ));
                self.saw_message_delta = true;
            }
        } else if let Some(usage) = chunk.get("usage") {
            // Final usage-only chunk (OpenAI stream_options.include_usage).
            // xAI often sends this *after* finish_reason; store for finish()/logs.
            let input_tokens = usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output_tokens = usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if input_tokens > 0 {
                self.last_input_tokens = input_tokens;
            }
            if output_tokens > 0 {
                self.last_output_tokens = output_tokens;
            }
            // If finish_reason already closed the message with zero usage, we cannot
            // emit a second message_delta safely; values remain for finish() fallback.
            let _ = (input_tokens, output_tokens);
        }

        out
    }

    fn ensure_block(&mut self, kind: BlockKind) -> Vec<u8> {
        let mut out = Vec::new();
        let need_new = match &self.current {
            Some((_, k)) if k == &kind && kind != BlockKind::ToolUse => false,
            Some(_) => true,
            None => true,
        };
        // Text/thinking: reuse same block if already open with same kind.
        if !need_new {
            return out;
        }
        // For text after text — keep same block (need_new false handled).
        // For switching kinds, close previous.
        if self.current.is_some() {
            out.extend_from_slice(&self.close_current_block());
        }
        let index = self.next_index;
        self.next_index += 1;
        let content_block = match kind {
            BlockKind::Text => json!({"type": "text", "text": ""}),
            BlockKind::Thinking => json!({"type": "thinking", "thinking": ""}),
            BlockKind::ToolUse => json!({"type": "tool_use", "id": "", "name": "", "input": {}}),
        };
        out.extend_from_slice(&encode_event(
            "content_block_start",
            &json!({
                "type": "content_block_start",
                "index": index,
                "content_block": content_block
            }),
        ));
        self.current = Some((index, kind));
        out
    }

    fn close_current_block(&mut self) -> Vec<u8> {
        if let Some((index, _)) = self.current.take() {
            encode_event(
                "content_block_stop",
                &json!({
                    "type": "content_block_stop",
                    "index": index
                }),
            )
        } else {
            Vec::new()
        }
    }
}

fn encode_event(event: &str, data: &Value) -> Vec<u8> {
    let payload = serde_json::to_string(data).unwrap_or_else(|_| "{}".into());
    format!("event: {event}\ndata: {payload}\n\n").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse_data(v: Value) -> String {
        format!("data: {}\n\n", serde_json::to_string(&v).unwrap())
    }

    #[test]
    fn streams_text_and_stop() {
        let mut s = OpenAiToAnthropicSse::new();
        let mut out = String::new();
        out.push_str(&String::from_utf8_lossy(&s.push(
            sse_data(json!({
                "id": "chatcmpl-x",
                "model": "grok-4.5",
                "choices": [{"index": 0, "delta": {"role": "assistant", "content": "He"}}]
            }))
            .as_bytes(),
        )));
        out.push_str(&String::from_utf8_lossy(&s.push(
            sse_data(json!({
                "id": "chatcmpl-x",
                "model": "grok-4.5",
                "choices": [{"index": 0, "delta": {"content": "llo"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 2}
            }))
            .as_bytes(),
        )));
        out.push_str(&String::from_utf8_lossy(
            &s.push(b"data: [DONE]\n\n"),
        ));

        assert!(out.contains("event: message_start"));
        assert!(out.contains("text_delta"));
        assert!(out.contains("Hello") || (out.contains("He") && out.contains("llo")));
        assert!(out.contains("end_turn"));
        assert!(out.contains("event: message_stop"));
    }

    #[test]
    fn streams_tool_use() {
        let mut s = OpenAiToAnthropicSse::new();
        let mut out = String::new();
        out.push_str(&String::from_utf8_lossy(&s.push(
            sse_data(json!({
                "id": "c1",
                "model": "grok-4.5",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_xyz",
                            "type": "function",
                            "function": {"name": "Bash", "arguments": ""}
                        }]
                    }
                }]
            }))
            .as_bytes(),
        )));
        out.push_str(&String::from_utf8_lossy(&s.push(
            sse_data(json!({
                "id": "c1",
                "model": "grok-4.5",
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "function": {"arguments": "{\"command\":\"ls\"}"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            }))
            .as_bytes(),
        )));
        out.push_str(&String::from_utf8_lossy(&s.push(b"data: [DONE]\n\n")));

        assert!(out.contains("tool_use"));
        assert!(out.contains("call_xyz"));
        assert!(out.contains("Bash"));
        assert!(out.contains("input_json_delta"));
        assert!(out.contains("tool_use")); // stop_reason
        assert!(out.contains("\"stop_reason\":\"tool_use\"") || out.contains("tool_use"));
    }

    #[test]
    fn finish_synthesizes_stop_after_partial_stream() {
        // Upstream abort mid-thinking: Claude Code needs message_stop, not a hard body error.
        // Use passthrough so thinking blocks still exercise the lifecycle.
        let mut s = OpenAiToAnthropicSse::with_thinking_mode(ThinkingMode::Passthrough);
        let mut out = String::new();
        out.push_str(&String::from_utf8_lossy(&s.push(
            sse_data(json!({
                "id": "chatcmpl-abort",
                "model": "grok-4.5",
                "choices": [{"index": 0, "delta": {
                    "role": "assistant",
                    "reasoning_content": "partial"
                }}]
            }))
            .as_bytes(),
        )));
        // No finish_reason / [DONE] — simulate upstream drop then finish().
        out.push_str(&String::from_utf8_lossy(&s.finish()));

        assert!(out.contains("event: message_start"));
        assert!(out.contains("thinking_delta"));
        assert!(out.contains("content_block_stop"));
        assert!(out.contains("event: message_delta"));
        assert!(out.contains("event: message_stop"));
        assert!(out.contains("end_turn"));
    }

    #[test]
    fn hide_thinking_skips_reasoning_deltas() {
        let mut s = OpenAiToAnthropicSse::with_thinking_mode(ThinkingMode::Hide);
        let mut out = String::new();
        out.push_str(&String::from_utf8_lossy(&s.push(
            sse_data(json!({
                "id": "c-hide",
                "model": "grok-4.5",
                "choices": [{"index": 0, "delta": {
                    "role": "assistant",
                    "reasoning_content": "secret",
                    "content": "hi"
                }}]
            }))
            .as_bytes(),
        )));
        out.push_str(&String::from_utf8_lossy(&s.finish()));
        assert!(!out.contains("thinking_delta"));
        assert!(out.contains("text_delta") || out.contains("\"text\":\"hi\"") || out.contains("hi"));
    }

    #[test]
    fn stores_usage_only_chunk_for_finish() {
        let mut s = OpenAiToAnthropicSse::new();
        let _ = s.push(
            sse_data(json!({
                "id": "c2",
                "model": "grok-4.5",
                "choices": [{"index": 0, "delta": {"content": "x"}}]
            }))
            .as_bytes(),
        );
        // Usage arrives without choices (include_usage trailing chunk), then abort.
        let _ = s.push(
            sse_data(json!({
                "id": "c2",
                "model": "grok-4.5",
                "choices": [],
                "usage": {"prompt_tokens": 12, "completion_tokens": 3}
            }))
            .as_bytes(),
        );
        let out = String::from_utf8_lossy(&s.finish()).into_owned();
        assert!(out.contains("\"input_tokens\":12") || out.contains("\"input_tokens\": 12"));
        assert!(out.contains("\"output_tokens\":3") || out.contains("\"output_tokens\": 3"));
        assert!(out.contains("message_stop"));
    }
}
