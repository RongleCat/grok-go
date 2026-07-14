import { invoke } from "@tauri-apps/api/core";

export type UsageSummary = {
  totalRequests: number;
  successRequests: number;
  inputTokens: number;
  outputTokens: number;
  cacheTokens: number;
  estimatedCostUsd: number;
};

export type AppStatus = {
  running: boolean;
  preferredPort: number;
  actualPort: number;
  bindHost: string;
  lanEnabled: boolean;
  requireToken: boolean;
  localToken: string;
  baseUrl: string;
  mcpUrl: string;
  accountCount: number;
  healthyAccounts: number;
  lanAddress?: string | null;
  today: UsageSummary;
};

export type AppConfig = {
  preferredPort: number;
  actualPort: number;
  bindHost: string;
  lanEnabled: boolean;
  requireToken: boolean;
  localToken: string;
  defaultModel: string;
  defaultImageModel: string;
  defaultVideoModel: string;
  modelMappings: Record<string, string>;
  routingStrategy:
    | "weighted-round-robin"
    | "least-recently-used"
    | "lowest-error-rate"
    | "fill-first";
  /** Stick multi-turn sessions to one account (default true). */
  sessionAffinity?: boolean;
  sessionAffinityTtlSecs?: number;
  /** Soft-weight by SuperGrok remaining / rate-limit (default true). */
  quotaAwareRouting?: boolean;
  /** Prefer accounts whose weekly quota resets soonest (default false). */
  preferSoonestReset?: boolean;
  /** Soft per-account in-flight preference cap; 0 = unlimited (default 6). */
  accountMaxConcurrency?: number;
  /**
   * Silently retry once when /v1/responses returns reasoning-only empty
   * completion (no message/tool call). Prevents Codex mid-task stop. Default true.
   */
  emptyCompletionRetry?: boolean;
  autoInjectCodexMcp: boolean;
  launchOnStartup: boolean;
  minimizeToTray: boolean;
  xaiClientId: string;
  xaiBaseUrl: string;
  oauthRedirectPort: number;
  /** When true, upstream xAI / OAuth requests use httpProxyUrl. Default false. */
  httpProxyEnabled: boolean;
  /** e.g. http://127.0.0.1:7890 or socks5://127.0.0.1:1080 */
  httpProxyUrl: string;
  /** Dock / window / tray brand: dark (default) or light. */
  appIcon: AppIconStyle;
  /**
   * MCP tools exposed via tools/list. null/undefined = all enabled.
   * Explicit array = only those tool names.
   */
  mcpEnabledTools?: string[] | null;
};

/** Canonical MCP tool ids (order matches gateway catalog). */
export const MCP_TOOL_CATALOG: {
  id: string;
  group: "search" | "image" | "video";
}[] = [
  { id: "x_search", group: "search" },
  { id: "image_gen", group: "image" },
  { id: "image_generate", group: "image" },
  { id: "image_edit", group: "image" },
  { id: "video_generate", group: "video" },
  { id: "video_edit", group: "video" },
];

export function isMcpToolEnabled(
  config: Pick<AppConfig, "mcpEnabledTools">,
  toolId: string
): boolean {
  if (config.mcpEnabledTools == null) return true;
  return config.mcpEnabledTools.includes(toolId);
}

export type AppIconStyle = "dark" | "light";

export type QuotaProductUsage = {
  productId: number;
  label: string;
  usedPercent: number;
};

export type AccountQuota = {
  usedPercent: number;
  remainingPercent: number;
  periodStartAt?: string | null;
  resetsAt?: string | null;
  products: QuotaProductUsage[];
  fetchedAt: string;
  lastError?: string | null;
};

export type Account = {
  id: string;
  name: string;
  email?: string | null;
  enabled: boolean;
  weight: number;
  /** oauth (default) or legacy sso (must convert to OAuth before routing). */
  authKind?: "oauth" | "sso";
  accessToken?: string | null;
  refreshToken?: string | null;
  /** Original card SSO JWT (kept after convert; not used for API). */
  ssoToken?: string | null;
  passwordHint?: string | null;
  /** Legacy field (unused). */
  ssoPool?: "basic" | "super" | "heavy";
  tokenType?: string | null;
  expiresAt?: string | null;
  lastRefresh?: string | null;
  lastSuccessAt?: string | null;
  lastFailureAt?: string | null;
  consecutiveFailures: number;
  health: "healthy" | "degraded" | "cooldown" | "disabled";
  cooldownUntil?: string | null;
  dailyLimitUsd?: number | null;
  monthlyLimitUsd?: number | null;
  notes?: string | null;
  /** From upstream x-ratelimit-* headers when present. */
  rateLimitLimit?: number | null;
  rateLimitRemaining?: number | null;
  rateLimitResetAt?: string | null;
  lastUpstreamError?: string | null;
  /** SuperGrok weekly credit quota (remaining + reset). */
  quota?: AccountQuota | null;
  /** Whether this account can handle image generation. Default true. */
  supportsImage?: boolean;
  /** Whether this account can handle video generation. Default true. */
  supportsVideo?: boolean;
};

export type ImportAccountsOptions = {
  weight?: number;
  supportsImage?: boolean;
  supportsVideo?: boolean;
  skipDuplicates?: boolean;
  validateRefresh?: boolean;
};

export type ImportAccountsResult = {
  added: number;
  skipped: number;
  failed: number;
  accounts: Account[];
  errors: { index: number; detail: string }[];
};

export type BatchAccountPatch = {
  enabled?: boolean | null;
  weight?: number | null;
  supportsImage?: boolean | null;
  supportsVideo?: boolean | null;
  clearCooldown?: boolean | null;
};

export type RequestLog = {
  requestId: string;
  accountId?: string | null;
  endpoint: string;
  requestedModel?: string | null;
  resolvedModel?: string | null;
  statusCode: number;
  latencyMs: number;
  firstTokenMs?: number | null;
  inputTokens: number;
  outputTokens: number;
  cacheTokens: number;
  estimatedCostUsd: number;
  errorSummary?: string | null;
  clientSource: string;
  createdAt: string;
};

export type HeatmapDay = {
  date: string;
  requests: number;
  tokens: number;
  costUsd: number;
};

export type IntegrationStatus = {
  codexMcpInjected: boolean;
  codexConfigPath: string;
  codexAgentsInjected: boolean;
  /** Codex global AGENTS.md (short reference only when injected). */
  codexAgentsPath: string;
  /** Versioned full guide under ~/.grok-go/agents-guide.md */
  agentsGuideFilePath: string;
  ccSwitchDbPath: string;
  /** Grok Build ~/.grok/config.toml has GrokGo model entry. */
  grokBuildInjected: boolean;
  grokBuildConfigPath: string;
  providerSnippet: string;
  mcpSnippet: string;
  grokBuildSnippet: string;
};

export type ModelOptions = {
  codex: string[];
  grokText: string[];
  grokImage: string[];
  grokVideo: string[];
};

export const api = {
  getStatus: () => invoke<AppStatus>("get_status"),
  startServer: () => invoke<AppStatus>("start_server"),
  getConfig: () => invoke<AppConfig>("get_config"),
  updateConfig: (config: AppConfig) => invoke<AppConfig>("update_config", { config }),
  setAppIcon: (style: AppIconStyle) => invoke<AppConfig>("set_app_icon", { style }),
  rotateToken: () => invoke<AppConfig>("rotate_token"),
  listModelOptions: () => invoke<ModelOptions>("list_model_options"),
  getAccounts: () => invoke<Account[]>("get_accounts"),
  upsertAccount: (account: Account) => invoke<Account[]>("upsert_account", { account }),
  deleteAccount: (accountId: string) => invoke<Account[]>("delete_account", { accountId }),
  replaceAccounts: (accounts: Account[]) => invoke<Account[]>("replace_accounts", { accounts }),
  importAccounts: (payload: string, options?: ImportAccountsOptions) =>
    invoke<ImportAccountsResult>("import_accounts", { payload, options }),
  /** Convert legacy SSO-only accounts → OAuth via Device Flow. */
  convertSsoAccounts: () => invoke<ImportAccountsResult>("convert_sso_accounts"),
  batchDeleteAccounts: (accountIds: string[]) =>
    invoke<Account[]>("batch_delete_accounts", { accountIds }),
  batchPatchAccounts: (accountIds: string[], patch: BatchAccountPatch) =>
    invoke<Account[]>("batch_patch_accounts", { accountIds, patch }),
  clearAccountCooldown: (accountId: string) =>
    invoke<Account[]>("clear_account_cooldown", { accountId }),
  refreshAccountQuota: (accountId: string) =>
    invoke<Account[]>("refresh_account_quota", { accountId }),
  refreshAllAccountQuotas: () => invoke<Account[]>("refresh_all_account_quotas"),
  startOAuthLogin: (opts?: { accountName?: string; accountId?: string }) =>
    invoke<{ accountId: string; authorizeUrl: string; browserOpened: boolean }>(
      "start_oauth_login",
      {
        // Use null so Tauri always receives Option::None (undefined keys can be dropped).
        accountName: opts?.accountName ?? null,
        accountId: opts?.accountId ?? null,
      }
    ),
  getUsageSummary: () => invoke<UsageSummary>("get_usage_summary"),
  getRecentLogs: (limit = 50, offset = 0) =>
    invoke<RequestLog[]>("get_recent_logs", { limit, offset }),
  getHeatmap: (days = 371) => invoke<HeatmapDay[]>("get_heatmap", { days }),
  clearLogs: () => invoke<void>("clear_logs"),
  getIntegrations: () => invoke<IntegrationStatus>("get_integrations"),
  setMcpInject: (enabled: boolean) => invoke<IntegrationStatus>("set_mcp_inject", { enabled }),
  injectAgentsGuide: () => invoke<IntegrationStatus>("inject_agents_guide"),
  setGrokBuildInject: (enabled: boolean) =>
    invoke<IntegrationStatus>("set_grok_build_inject", { enabled }),
  importToCcSwitch: () => invoke<string>("import_to_cc_switch"),
  exportProviderSnippet: () => invoke<string>("export_provider_snippet"),
};
