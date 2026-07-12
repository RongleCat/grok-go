# Grok Proxy Desktop Design

**Date:** 2026-07-10  
**Status:** Approved and implementation started  
**Platforms:** macOS, Windows

## Goal

Build a Tauri desktop control console that logs into xAI via OAuth and exposes Grok capabilities to Codex and other tools through:

- local Responses proxy
- OpenAI-compatible proxy
- native image generation endpoints
- MCP tools (`x_search`, image, video)
- multi-account hosting + load balancing
- usage analytics and request logs
- CC Switch provider import
- optional Codex MCP auto-inject/cleanup

## Product Principles

1. Codex primary path is Responses (`wire_api = "responses"`).
2. Proxy prefers transparent pass-through plus auth injection.
3. Credentials are stored in migratable config files, not OS keychains.
4. Multi-account routing is first-class.
5. UI uses shadcn-style components + Emil Kowalski design guidance.
6. Do not mutate unrelated Codex/CC Switch configuration.

## Architecture

```text
Tauri Desktop App
├── React UI
└── Local Gateway Runtime
    ├── Auth Pool (multi-account OAuth PKCE)
    ├── Router / Load Balancer
    ├── Responses Proxy
    ├── Chat Completions Proxy
    ├── Images Proxy
    ├── MCP Server
    └── Usage Logger + Stats Aggregator
```

## Storage Layout

```text
~/.grok-proxy/
  config.json
  auth.json
  data.db
  artifacts/
  logs/
  backups/
```

## Local API Surface

Preferred port: `8787` with auto-increment.

- `POST /v1/responses`
- `POST /v1/chat/completions`
- `GET /v1/models`
- `POST /v1/images/generations`
- `POST /v1/images/edits`
- MCP: `/mcp`

Tools:

- `x_search`
- `image_generate`
- `image_edit`
- `video_generate`
- `video_edit`

## Model Mapping

1. exact mapping hit
2. pass-through valid Grok text model
3. fallback to default model

## Integrations

- Codex MCP inject only manages `mcp_servers.grok-proxy`
- Provider is not auto-injected into Codex
- CC Switch provider import is supported

## Implementation Status

Implemented in current codebase:

- Tauri 2 project scaffold
- Rust gateway runtime
- OAuth multi-account flow
- Proxy endpoints
- MCP endpoint
- usage DB + heatmap API
- React control console pages
