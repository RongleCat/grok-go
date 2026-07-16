//! Anthropic Messages API compatibility for Claude Code.
//!
//! Converts:
//! - Request:  Anthropic Messages → OpenAI Chat Completions (xAI upstream)
//! - Response: OpenAI Chat Completions → Anthropic Messages
//! - Stream:   OpenAI SSE → Anthropic SSE (`message_*` / `content_block_*`)
//!
//! Design notes (see `llm-wiki/wiki/queries/anthropic-claude-code-research.md`):
//! - Self-contained (no external protocol crate) so GrokGo owns MSRV and tool-call fidelity.
//! - Logic aligned with mature open sources: llm-bridge-core, anthropic-proxy-rs, cc-switch.
//! - Critical: preserve `tool_use.id` ↔ `tool_calls[].id` and emit proper stream block lifecycle.

mod request;
mod response;
mod schema;
mod stream;

pub use request::{anthropic_to_openai_chat, estimate_token_count, map_client_model};
pub use response::{
    anthropic_error_body, openai_chat_to_anthropic, openai_chat_to_anthropic_with_thinking,
    openai_error_to_anthropic, ThinkingMode,
};
// openai_chat_to_anthropic re-exported for API stability (default hide).
pub use stream::OpenAiToAnthropicSse;
