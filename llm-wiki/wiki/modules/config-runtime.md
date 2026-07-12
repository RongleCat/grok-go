# 模块：配置与运行时路径

## 结论

配置模型在 `config.rs`，路径解析在 `paths.rs`。热路径使用内存 cache，避免每次请求读盘。

## AppConfig 默认值（摘要）

| 字段 | 默认 |
|---|---|
| preferred_port / actual_port | 8787 |
| bind_host | 127.0.0.1 |
| lan_enabled | false |
| require_token | true |
| local_token | 随机 40 字符 |
| default_model | grok-4.5 |
| default_image_model | grok-imagine-image-quality |
| default_video_model | grok-imagine-video |
| model_mappings | gpt-5.6/gpt-5.5 → grok-4.5 |
| routing_strategy | weighted-round-robin |
| auto_inject_codex_mcp | false |
| launch_on_startup | false |
| minimize_to_tray | true |
| xai_base_url | https://api.x.ai/v1 |
| oauth_redirect_port | 56121 |
| http_proxy_enabled | false |
| app_icon | dark |
| mcp_enabled_tools | null（全部） |

## 路径 API

| 函数 | 路径 |
|---|---|
| `app_home` | `~/.grok-go` |
| `config_path` | `.../config.json` |
| `auth_path` | `.../auth.json` |
| `db_path` | `.../data.db` |
| `artifacts_dir` | `.../artifacts` |
| `agents_guide_file_path` | `.../agents-guide.md` |
| `codex_config_path` | `~/.codex/config.toml` |
| `codex_agents_md_path` | `~/.codex/AGENTS.md` |
| `cc_switch_db_path` | `~/.cc-switch/cc-switch.db` |
| `grok_build_config_path` | `~/.grok/config.toml` |

## 缓存

- `CONFIG_CACHE` / `AUTH_CACHE`（parking_lot RwLock）
- `load_*` 读 cache；`save_*` 写盘并更新 cache
- 成功热路径可用 `patch_account_cache` 避免频繁写 auth

## 相关页面

- [[../concepts/model-mapping]]
- [[auth-oauth]]
- [[routing]]

## 来源

- `src-tauri/src/config.rs`
- `src-tauri/src/paths.rs`
