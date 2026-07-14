import { useCallback, useEffect, useMemo, useState, type MouseEvent } from "react";
import { Copy, FileText, Plug, RefreshCw, RotateCcw, Shield } from "lucide-react";
import {
  api,
  isMcpToolEnabled,
  MCP_TOOL_CATALOG,
  type AppConfig,
  type IntegrationStatus,
} from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog } from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { Tabs } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { useToast } from "@/components/ui/toast";
import { useI18n } from "@/i18n/context";
import { PageBody, PageHeader, PageShell } from "@/components/page-shell";
import { PageLoading } from "@/components/page-loading";
import { cn } from "@/lib/utils";

type TabId = "codex" | "claude-code" | "mcp" | "clients" | "grok-build";

/** Grok Build native multi-account routing (cli-chat-proxy). */
const SHOW_GROK_BUILD_TAB = true;

type ClientSnippet = {
  id: string;
  title: string;
  format: string;
  body: string;
};

function mcpBaseUrl(config: AppConfig): string {
  const host = config.lanEnabled
    ? config.bindHost === "0.0.0.0" || config.bindHost === "127.0.0.1"
      ? "127.0.0.1"
      : config.bindHost
    : "127.0.0.1";
  return `http://${host}:${config.actualPort || config.preferredPort}/mcp`;
}

function authHeaderBlock(config: AppConfig): { json: Record<string, string>; toml: string } {
  if (!config.requireToken || !config.localToken?.trim()) {
    return { json: {}, toml: "" };
  }
  const token = config.localToken.trim();
  return {
    json: { Authorization: `Bearer ${token}` },
    toml: `\n\n[mcp_servers.grok-go.http_headers]\nAuthorization = "Bearer ${token}"\n`,
  };
}

function buildClientSnippets(config: AppConfig, t: ReturnType<typeof useI18n>["t"]): ClientSnippet[] {
  const url = mcpBaseUrl(config);
  const { json: headers, toml: tomlHeaders } = authHeaderBlock(config);
  const hasAuth = Object.keys(headers).length > 0;

  const vscodeCursor = {
    mcpServers: {
      "grok-go": {
        url,
        ...(hasAuth ? { headers } : {}),
      },
    },
  };

  const cherry = {
    mcpServers: {
      "grok-go": {
        type: "streamableHttp",
        url,
        ...(hasAuth ? { headers } : {}),
      },
    },
  };

  const claudeDesktop = {
    mcpServers: {
      "grok-go": {
        transport: "streamable-http",
        url,
        ...(hasAuth ? { headers } : {}),
      },
    },
  };

  const codexToml = `[mcp_servers.grok-go]\nurl = "${url}"${tomlHeaders}`;

  const genericJson = JSON.stringify(
    {
      name: "grok-go",
      url,
      ...(hasAuth ? { headers } : {}),
    },
    null,
    2
  );

  return [
    {
      id: "vscode",
      title: t.integrations.clientVscode,
      format: "JSON · settings.json / mcp.json",
      body: JSON.stringify(vscodeCursor, null, 2),
    },
    {
      id: "cursor",
      title: t.integrations.clientCursor,
      format: "JSON · .cursor/mcp.json",
      body: JSON.stringify(vscodeCursor, null, 2),
    },
    {
      id: "cherry",
      title: t.integrations.clientCherry,
      format: "JSON · streamableHttp",
      body: JSON.stringify(cherry, null, 2),
    },
    {
      id: "claude",
      title: t.integrations.clientClaude,
      format: "JSON · Claude Desktop",
      body: JSON.stringify(claudeDesktop, null, 2),
    },
    {
      id: "codex",
      title: t.integrations.clientCodex,
      format: "TOML · ~/.codex/config.toml",
      body: codexToml,
    },
    {
      id: "generic",
      title: t.integrations.clientGeneric,
      format: "JSON",
      body: genericJson,
    },
  ];
}

export function IntegrationsPage() {
  const { t } = useI18n();
  const { toast } = useToast();
  const [tab, setTab] = useState<TabId>("codex");
  const [status, setStatus] = useState<IntegrationStatus | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [toolDraft, setToolDraft] = useState<Record<string, boolean>>({});
  const [toolsDirty, setToolsDirty] = useState(false);
  const [preview, setPreview] = useState<ClientSnippet | null>(null);

  const load = useCallback(async () => {
    const [st, cfg] = await Promise.all([api.getIntegrations(), api.getConfig()]);
    setStatus(st);
    setConfig(cfg);
    const draft: Record<string, boolean> = {};
    for (const tool of MCP_TOOL_CATALOG) {
      draft[tool.id] = isMcpToolEnabled(cfg, tool.id);
    }
    setToolDraft(draft);
    setToolsDirty(false);
    setError("");
  }, []);

  useEffect(() => {
    load().catch((e) => setError(String(e)));
  }, [load]);

  const tabs = useMemo(() => {
    const list: { id: TabId; label: string }[] = [
      { id: "codex", label: t.integrations.tabCodex },
      { id: "claude-code", label: t.integrations.tabClaudeCode },
      { id: "mcp", label: t.integrations.tabMcp },
      { id: "clients", label: t.integrations.tabClients },
    ];
    if (SHOW_GROK_BUILD_TAB) {
      list.push({ id: "grok-build", label: t.integrations.tabGrokBuild });
    }
    return list;
  }, [t]);

  const claudeBaseUrl = useMemo(() => {
    if (!config) return "http://127.0.0.1:8787";
    const host = config.lanEnabled ? "0.0.0.0" : "127.0.0.1";
    // Prefer actualPort when set; UI status often has the live port in gateway.
    const port = config.actualPort || config.preferredPort || 8787;
    // Match backend anthropic_base_url (no /v1). When LAN, backend uses LAN IP;
    // snippet from status is authoritative when available.
    if (status?.claudeCodeSnippet) {
      try {
        const parsed = JSON.parse(status.claudeCodeSnippet) as {
          env?: { ANTHROPIC_BASE_URL?: string };
        };
        if (parsed.env?.ANTHROPIC_BASE_URL) return parsed.env.ANTHROPIC_BASE_URL;
      } catch {
        /* ignore */
      }
    }
    return `http://${host === "0.0.0.0" ? "127.0.0.1" : host}:${port}`;
  }, [config, status]);

  const clients = useMemo(
    () => (config ? buildClientSnippets(config, t) : []),
    [config, t]
  );

  async function copyText(label: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast(`${label} · ${t.common.copied}`, "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function saveMcpTools() {
    if (!config) return;
    setBusy("tools");
    try {
      const enabled = MCP_TOOL_CATALOG.filter((tool) => toolDraft[tool.id]).map((tool) => tool.id);
      const next = await api.updateConfig({
        ...config,
        mcpEnabledTools: enabled,
      });
      setConfig(next);
      setToolsDirty(false);
      toast(t.integrations.toolsSaved, "success");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(null);
    }
  }

  if (!status && !error) return <PageLoading />;
  if (!status || !config) {
    return (
      <div className="space-y-3">
        <div className="text-sm text-red-600">
          {t.common.loadFailed}
          {error ? `：${error}` : ""}
        </div>
        <Button
          variant="outline"
          onClick={() => load().catch((e) => setError(String(e)))}
        >
          {t.common.retry}
        </Button>
      </div>
    );
  }

  return (
    <PageShell>
      <PageHeader>
        <div className="min-w-0 space-y-1">
          <h1 className="text-xl font-semibold tracking-tight">{t.integrations.title}</h1>
          {t.integrations.subtitle ? (
            <p className="text-sm text-neutral-500">{t.integrations.subtitle}</p>
          ) : null}
        </div>
        <Button
          size="sm"
          variant="outline"
          disabled={busy !== null}
          onClick={() => {
            void load()
              .then(() => toast(t.common.refresh, "info"))
              .catch((e) => toast(String(e), "error"));
          }}
        >
          <RefreshCw className="h-3.5 w-3.5" />
          {t.common.refresh}
        </Button>
      </PageHeader>

      <div className="shrink-0">
        <Tabs items={tabs} value={tab} onChange={setTab} />
      </div>

      <PageBody className="space-y-4 pb-2">
        {tab === "codex" && (
          <>
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Plug className="h-4 w-4 text-neutral-500" />
                  {t.integrations.sectionCodex}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-0 divide-y divide-neutral-100 p-0">
                <div className="flex items-start justify-between gap-4 px-4 py-3">
                  <div className="min-w-0 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-medium">{t.integrations.mcpInject}</span>
                      <Badge variant={status.codexMcpInjected ? "success" : "outline"}>
                        {status.codexMcpInjected
                          ? t.integrations.injected
                          : t.integrations.notInjected}
                      </Badge>
                    </div>
                    {t.integrations.mcpInjectDesc ? (
                      <p className="text-xs text-neutral-500">{t.integrations.mcpInjectDesc}</p>
                    ) : null}
                    <p className="truncate font-mono text-[11px] text-neutral-400">
                      {status.codexConfigPath}
                    </p>
                  </div>
                  <Switch
                    checked={status.codexMcpInjected}
                    disabled={busy !== null}
                    onCheckedChange={async (v) => {
                      setBusy("mcp");
                      try {
                        const hadAgentsGuide = status.codexAgentsInjected;
                        const next = await api.setMcpInject(v);
                        setStatus(next);
                        toast(
                          v
                            ? t.integrations.injectedMsg
                            : hadAgentsGuide
                              ? t.integrations.agentsGuideClearedWithMcp
                              : t.integrations.removedMsg,
                          "success"
                        );
                      } catch (e) {
                        toast(String(e), "error");
                      } finally {
                        setBusy(null);
                      }
                    }}
                  />
                </div>

                <div className="flex items-start justify-between gap-4 px-4 py-3">
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-medium">{t.integrations.agentsGuide}</span>
                      <Badge variant={status.codexAgentsInjected ? "success" : "outline"}>
                        {status.codexAgentsInjected
                          ? t.integrations.agentsGuideInjected
                          : t.integrations.agentsGuideNotInjected}
                      </Badge>
                    </div>
                    {t.integrations.agentsGuideDesc ? (
                      <p className="text-xs text-neutral-500">{t.integrations.agentsGuideDesc}</p>
                    ) : null}
                    <div className="space-y-0.5 font-mono text-[11px] text-neutral-400">
                      <div className="truncate">
                        <span className="text-neutral-500">{t.integrations.agentsGuideFile} · </span>
                        {status.agentsGuideFilePath || "—"}
                      </div>
                      <div className="truncate">
                        <span className="text-neutral-500">
                          {t.integrations.agentsGuideAgentsMd} ·{" "}
                        </span>
                        {status.codexAgentsPath}
                      </div>
                    </div>
                  </div>
                  <Button
                    size="sm"
                    className="shrink-0"
                    disabled={busy !== null}
                    onClick={async () => {
                      setBusy("agents");
                      try {
                        const next = await api.injectAgentsGuide();
                        setStatus(next);
                        toast(t.integrations.agentsGuideMsg, "success");
                      } catch (e) {
                        toast(String(e), "error");
                      } finally {
                        setBusy(null);
                      }
                    }}
                  >
                    <FileText className="h-3.5 w-3.5" />
                    {t.integrations.agentsGuideInject}
                  </Button>
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="py-3">
                <CardTitle className="text-base">{t.integrations.ccSwitch}</CardTitle>
              </CardHeader>
              <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="min-w-0 space-y-1">
                  {t.integrations.ccSwitchDesc ? (
                    <p className="text-sm text-neutral-500">{t.integrations.ccSwitchDesc}</p>
                  ) : null}
                  <p className="truncate font-mono text-[11px] text-neutral-400">
                    {status.ccSwitchDbPath}
                  </p>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  className="shrink-0"
                  disabled={busy !== null}
                  onClick={async () => {
                    setBusy("cc");
                    try {
                      const msg = await api.importToCcSwitch();
                      toast(msg?.trim() || t.overview.importCcSwitchSuccess, "success");
                      await load();
                    } catch (e) {
                      const raw = String(e);
                      const cleaned = raw
                        .replace(/^Error:\s*/i, "")
                        .replace(/^.*failed to.*?[:：]\s*/i, "")
                        .trim();
                      toast(
                        cleaned
                          ? `${t.overview.importCcSwitchFailed}\n${cleaned}`
                          : t.overview.importCcSwitchFailed,
                        "error"
                      );
                    } finally {
                      setBusy(null);
                    }
                  }}
                >
                  {t.integrations.importCcSwitch}
                </Button>
              </CardContent>
            </Card>
          </>
        )}

        {tab === "claude-code" && (
          <>
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Plug className="h-4 w-4 text-neutral-500" />
                  {t.integrations.sectionClaudeCode}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                {t.integrations.claudeCodeDesc ? (
                  <p className="text-sm text-neutral-600 dark:text-neutral-400">
                    {t.integrations.claudeCodeDesc}
                  </p>
                ) : null}
                <div className="space-y-1 rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2.5 dark:border-neutral-800 dark:bg-neutral-900/40">
                  <div className="text-xs font-medium text-neutral-500">
                    {t.integrations.claudeCodeBaseUrl}
                  </div>
                  <div className="truncate font-mono text-sm">{claudeBaseUrl}</div>
                  {t.integrations.claudeCodeBaseUrlHint ? (
                    <p className="text-[11px] text-neutral-400">
                      {t.integrations.claudeCodeBaseUrlHint}
                    </p>
                  ) : null}
                </div>
                {t.integrations.claudeCodeRestartHint ? (
                  <p className="text-xs text-amber-600/90 dark:text-amber-400/90">
                    {t.integrations.claudeCodeRestartHint}
                  </p>
                ) : null}
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="py-3">
                <CardTitle className="text-base">{t.integrations.ccSwitch}</CardTitle>
              </CardHeader>
              <CardContent className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                <div className="min-w-0 space-y-1">
                  <p className="truncate font-mono text-[11px] text-neutral-400">
                    {status.ccSwitchDbPath}
                  </p>
                </div>
                <Button
                  size="sm"
                  className="shrink-0"
                  disabled={busy !== null}
                  onClick={async () => {
                    setBusy("cc-claude");
                    try {
                      const msg = await api.importClaudeToCcSwitch();
                      toast(msg?.trim() || t.overview.importCcSwitchSuccess, "success");
                      await load();
                    } catch (e) {
                      const raw = String(e);
                      const cleaned = raw
                        .replace(/^Error:\s*/i, "")
                        .replace(/^.*failed to.*?[:：]\s*/i, "")
                        .trim();
                      toast(
                        cleaned
                          ? `${t.overview.importCcSwitchFailed}\n${cleaned}`
                          : t.overview.importCcSwitchFailed,
                        "error"
                      );
                    } finally {
                      setBusy(null);
                    }
                  }}
                >
                  {t.integrations.importClaudeCcSwitch}
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="py-3">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="space-y-1">
                    <CardTitle className="text-base">
                      {t.integrations.claudeCodeSnippet}
                    </CardTitle>
                    {t.integrations.claudeCodeSnippetDesc ? (
                      <p className="text-sm text-neutral-500">
                        {t.integrations.claudeCodeSnippetDesc}
                      </p>
                    ) : null}
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={!status.claudeCodeSnippet}
                    onClick={() =>
                      void copyText(
                        t.integrations.claudeCodeSnippet,
                        status.claudeCodeSnippet || ""
                      )
                    }
                  >
                    <Copy className="h-3.5 w-3.5" />
                    {t.common.copy}
                  </Button>
                </div>
              </CardHeader>
              <CardContent>
                <Textarea
                  readOnly
                  className="min-h-[200px] font-mono text-xs"
                  value={status.claudeCodeSnippet || ""}
                />
              </CardContent>
            </Card>
          </>
        )}

        {tab === "mcp" && (
          <Card>
            <CardHeader className="py-3">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="space-y-1">
                  <CardTitle className="text-base">{t.integrations.toolsTitle}</CardTitle>
                  {t.integrations.toolsDesc ? (
                    <p className="text-sm text-neutral-500">{t.integrations.toolsDesc}</p>
                  ) : null}
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      const next: Record<string, boolean> = {};
                      for (const tool of MCP_TOOL_CATALOG) next[tool.id] = true;
                      setToolDraft(next);
                      setToolsDirty(true);
                    }}
                  >
                    {t.integrations.toolsSelectAll}
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      const next: Record<string, boolean> = {};
                      for (const tool of MCP_TOOL_CATALOG) next[tool.id] = false;
                      setToolDraft(next);
                      setToolsDirty(true);
                    }}
                  >
                    {t.integrations.toolsSelectNone}
                  </Button>
                  <Button
                    size="sm"
                    disabled={!toolsDirty || busy !== null}
                    onClick={() => void saveMcpTools()}
                  >
                    {t.integrations.toolsSave}
                  </Button>
                </div>
              </div>
            </CardHeader>
            <CardContent className="space-y-2">
              {MCP_TOOL_CATALOG.map((tool) => {
                const enabled = !!toolDraft[tool.id];
                const labels = toolLabels(t, tool.id);
                const title = labels.title;
                const desc = labels.desc;
                return (
                  <label
                    key={tool.id}
                    className={cn(
                      "flex cursor-pointer items-start gap-3 rounded-lg border px-3 py-2.5 transition",
                      enabled
                        ? "border-neutral-900/20 bg-neutral-50"
                        : "border-neutral-200 hover:border-neutral-300"
                    )}
                  >
                    <input
                      type="checkbox"
                      className="mt-1"
                      checked={enabled}
                      onChange={(e) => {
                        setToolDraft((prev) => ({ ...prev, [tool.id]: e.target.checked }));
                        setToolsDirty(true);
                      }}
                    />
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-mono text-sm font-medium">{tool.id}</span>
                        <Badge variant="outline" className="text-[10px]">
                          {title}
                        </Badge>
                      </div>
                      {desc ? <p className="mt-0.5 text-xs text-neutral-500">{desc}</p> : null}
                    </div>
                  </label>
                );
              })}
              {toolsDirty ? (
                <p className="text-xs text-amber-700">{t.integrations.toolsDirtyHint}</p>
              ) : null}
            </CardContent>
          </Card>
        )}

        {tab === "clients" && (
          <div className="grid gap-3 sm:grid-cols-2">
            {clients.map((client) => (
              <ClientCard
                key={client.id}
                client={client}
                copyLabel={t.common.copy}
                onCopy={(e) => {
                  e.stopPropagation();
                  void copyText(client.title, client.body);
                }}
                onOpen={() => setPreview(client)}
              />
            ))}
          </div>
        )}

        {SHOW_GROK_BUILD_TAB && tab === "grok-build" && (
          <div className="space-y-3">
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="flex flex-wrap items-center gap-2 text-base">
                  {t.integrations.grokBuild}
                  <Badge variant={status.grokBuildInjected ? "success" : "outline"}>
                    {status.grokBuildInjected
                      ? t.integrations.grokBuildInjected
                      : t.integrations.grokBuildNotInjected}
                  </Badge>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0 space-y-1.5">
                    {t.integrations.grokBuildDesc ? (
                      <p className="text-sm text-neutral-600 dark:text-neutral-400">
                        {t.integrations.grokBuildDesc}
                      </p>
                    ) : null}
                    {t.integrations.grokBuildRestartHint ? (
                      <p className="text-xs text-amber-600/90 dark:text-amber-400/90">
                        {t.integrations.grokBuildRestartHint}
                      </p>
                    ) : null}
                  </div>
                  <div className="flex shrink-0 flex-col items-end gap-2">
                    <Switch
                      checked={status.grokBuildInjected}
                      disabled={busy !== null}
                      onCheckedChange={async (v) => {
                        setBusy("grok-build");
                        try {
                          const next = await api.setGrokBuildInject(v);
                          setStatus(next);
                          toast(
                            v
                              ? t.integrations.grokBuildInjectedMsg
                              : t.integrations.grokBuildRemovedMsg,
                            "success"
                          );
                        } catch (e) {
                          toast(String(e), "error");
                        } finally {
                          setBusy(null);
                        }
                      }}
                    />
                  </div>
                </div>

                <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
                  <div className="rounded-lg border border-neutral-200/80 bg-neutral-50/80 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
                    <div className="text-[11px] uppercase tracking-wide text-neutral-400">
                      {t.integrations.grokBuildProtocol}
                    </div>
                    <div className="mt-0.5 text-sm font-medium text-neutral-800 dark:text-neutral-100">
                      {status.grokBuildProtocol || "cli-chat-proxy"}
                    </div>
                  </div>
                  <div className="rounded-lg border border-neutral-200/80 bg-neutral-50/80 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
                    <div className="text-[11px] uppercase tracking-wide text-neutral-400">
                      {t.integrations.grokBuildAccounts}
                    </div>
                    <div className="mt-0.5 text-sm font-medium text-neutral-800 dark:text-neutral-100">
                      {status.grokBuildAccountCount ?? 0}
                    </div>
                    {t.integrations.grokBuildAccountsHint ? (
                      <div className="mt-0.5 text-[11px] text-neutral-500">
                        {t.integrations.grokBuildAccountsHint}
                      </div>
                    ) : null}
                  </div>
                  <div className="rounded-lg border border-neutral-200/80 bg-neutral-50/80 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
                    <div className="text-[11px] uppercase tracking-wide text-neutral-400">
                      {t.integrations.grokBuildSession}
                    </div>
                    <div className="mt-0.5 truncate text-sm font-medium text-neutral-800 dark:text-neutral-100">
                      {status.grokBuildSessionEmail || t.integrations.grokBuildSessionEmpty}
                    </div>
                    <div className="mt-0.5 text-[11px] text-neutral-500">
                      {t.integrations.grokBuildSessionTier}:{" "}
                      {status.grokBuildSessionTier != null ? status.grokBuildSessionTier : "—"}
                      {status.grokBuildSessionReferrer
                        ? ` · ${t.integrations.grokBuildSessionReferrer}: ${status.grokBuildSessionReferrer}`
                        : ""}
                    </div>
                  </div>
                  <div className="rounded-lg border border-neutral-200/80 bg-neutral-50/80 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
                    <div className="flex items-center gap-1 text-[11px] uppercase tracking-wide text-neutral-400">
                      <Shield className="h-3 w-3" />
                      Backup
                    </div>
                    <div className="mt-0.5 text-sm font-medium text-neutral-800 dark:text-neutral-100">
                      {status.grokBuildRestoreAvailable
                        ? t.integrations.grokBuildInjected
                        : t.integrations.grokBuildRestoreUnavailable}
                    </div>
                    {t.integrations.grokBuildBackupHint ? (
                      <div className="mt-0.5 text-[11px] text-neutral-500">
                        {t.integrations.grokBuildBackupHint}
                      </div>
                    ) : null}
                  </div>
                </div>

                {status.grokBuildSessionWarn ? (
                  <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200/90">
                    <div className="font-medium">{t.integrations.grokBuildSessionWarnTitle}</div>
                    <p className="mt-1 whitespace-pre-wrap leading-relaxed">
                      {status.grokBuildSessionWarn}
                    </p>
                  </div>
                ) : null}

                <div className="space-y-1">
                  <p className="truncate font-mono text-[11px] text-neutral-400">
                    {status.grokBuildConfigPath}
                  </p>
                  <p className="truncate font-mono text-[11px] text-neutral-400">
                    {status.grokBuildAuthPath}
                  </p>
                </div>

                <div className="flex flex-wrap gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy !== null || !status.grokBuildRestoreAvailable}
                    onClick={async () => {
                      setBusy("grok-build-restore");
                      try {
                        const next = await api.restoreGrokBuildBackup();
                        setStatus(next);
                        toast(t.integrations.grokBuildRestoreMsg, "success");
                      } catch (e) {
                        toast(String(e), "error");
                      } finally {
                        setBusy(null);
                      }
                    }}
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                    {t.integrations.grokBuildRestore}
                  </Button>
                  {status.grokBuildSnippet ? (
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() =>
                        void copyText(t.integrations.grokBuild, status.grokBuildSnippet)
                      }
                    >
                      <Copy className="h-3.5 w-3.5" />
                      {t.common.copy}
                    </Button>
                  ) : null}
                </div>
              </CardContent>
            </Card>
          </div>
        )}

      <Dialog
        open={!!preview}
        title={preview?.title ?? ""}
        description={preview?.format}
        onClose={() => setPreview(null)}
        className="max-w-2xl"
      >
        {preview ? (
          <div className="space-y-3">
            <Textarea
              readOnly
              value={preview.body}
              className="min-h-[280px] font-mono text-xs"
            />
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setPreview(null)}>
                {t.common.cancel}
              </Button>
              <Button
                onClick={() => {
                  void copyText(preview.title, preview.body);
                }}
              >
                <Copy className="h-3.5 w-3.5" />
                {t.common.copy}
              </Button>
            </div>
          </div>
        ) : null}
      </Dialog>
      </PageBody>
    </PageShell>
  );
}

function toolLabels(
  t: ReturnType<typeof useI18n>["t"],
  id: string
): { title: string; desc: string } {
  const i = t.integrations;
  switch (id) {
    case "x_search":
      return { title: i.tool_x_search, desc: i.tool_x_search_desc };
    case "image_gen":
      return { title: i.tool_image_gen, desc: i.tool_image_gen_desc };
    case "image_generate":
      return { title: i.tool_image_generate, desc: i.tool_image_generate_desc };
    case "image_edit":
      return { title: i.tool_image_edit, desc: i.tool_image_edit_desc };
    case "video_generate":
      return { title: i.tool_video_generate, desc: i.tool_video_generate_desc };
    case "video_edit":
      return { title: i.tool_video_edit, desc: i.tool_video_edit_desc };
    default:
      return { title: id, desc: "" };
  }
}

function ClientCard({
  client,
  copyLabel,
  onCopy,
  onOpen,
}: {
  client: ClientSnippet;
  copyLabel: string;
  onCopy: (e: MouseEvent) => void;
  onOpen: () => void;
}) {
  return (
    <Card
      className="cursor-pointer transition hover:border-neutral-400 hover:shadow-sm"
      onClick={onOpen}
    >
      <CardContent className="flex items-center gap-3 p-4">
        <div className="min-w-0 flex-1 text-sm font-semibold">{client.title}</div>
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="shrink-0"
          title={copyLabel}
          onClick={onCopy}
        >
          <Copy className="h-3.5 w-3.5" />
          {copyLabel}
        </Button>
      </CardContent>
    </Card>
  );
}
