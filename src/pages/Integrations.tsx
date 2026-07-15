import { useCallback, useEffect, useMemo, useState, type MouseEvent } from "react";
import { Copy, FileText, Plug, RefreshCw, RotateCcw } from "lucide-react";
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
import { CopyField } from "@/components/ui/copy-field";
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
    const port = config.actualPort || config.preferredPort || 8787;
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

  async function runInject(
    key: string,
    action: () => Promise<IntegrationStatus>,
    okMsg: string
  ) {
    setBusy(key);
    try {
      const next = await action();
      setStatus(next);
      toast(okMsg, "success");
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
              <CardContent className="space-y-3">
                {t.integrations.claudeCodeDesc ? (
                  <p className="text-sm text-neutral-500">{t.integrations.claudeCodeDesc}</p>
                ) : null}
                <CopyField
                  label={t.integrations.claudeCodeBaseUrl}
                  value={claudeBaseUrl}
                  copyLabel={t.common.copy}
                  onCopy={() =>
                    void copyText(t.integrations.claudeCodeBaseUrl, claudeBaseUrl)
                  }
                />
                {t.integrations.claudeCodeRestartHint ? (
                  <p className="text-xs text-neutral-500">{t.integrations.claudeCodeRestartHint}</p>
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
          <div className="flex min-h-0 flex-1 flex-col gap-3 lg:flex-row lg:items-stretch">
            <Card className="min-h-0 min-w-0 flex-1 lg:max-w-[58%]">
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
              <CardContent className="max-h-[min(60vh,520px)] space-y-0 divide-y divide-neutral-100 overflow-y-auto p-0">
                {MCP_TOOL_CATALOG.map((tool) => {
                  const enabled = !!toolDraft[tool.id];
                  const labels = toolLabels(t, tool.id);
                  const title = labels.title;
                  const desc = labels.desc;
                  return (
                    <label
                      key={tool.id}
                      className="flex cursor-pointer items-start gap-3 px-4 py-3 transition hover:bg-neutral-50"
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
                  <p className="px-4 py-2 text-xs text-neutral-500">{t.integrations.toolsDirtyHint}</p>
                ) : null}
              </CardContent>
            </Card>

            <Card className="flex min-h-0 min-w-0 flex-1 flex-col lg:max-w-[42%]">
              <CardHeader className="shrink-0 py-3">
                <CardTitle className="text-base">{t.integrations.clientSnippetsTitle}</CardTitle>
              </CardHeader>
              <CardContent className="min-h-0 flex-1 overflow-y-auto p-3 pt-0">
                <div className="flex flex-col gap-2">
                  {clients.map((client) => (
                    <ClientCard
                      key={client.id}
                      client={client}
                      copyLabel={t.common.copy}
                      compact
                      onCopy={(e) => {
                        e.stopPropagation();
                        void copyText(client.title, client.body);
                      }}
                      onOpen={() => setPreview(client)}
                    />
                  ))}
                </div>
              </CardContent>
            </Card>
          </div>
        )}

        {tab === "clients" && (
          <div className="space-y-3">
            {/* OpenCode */}
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="flex flex-wrap items-center gap-2 text-base">
                  {t.integrations.clientOpenCode}
                  <Badge
                    variant={
                      status.opencodeModelInjected || status.opencodeMcpInjected
                        ? "success"
                        : "outline"
                    }
                  >
                    {status.opencodeModelInjected || status.opencodeMcpInjected
                      ? t.integrations.injected
                      : t.integrations.notInjected}
                  </Badge>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-0 divide-y divide-neutral-100 p-0">
                <InjectRow
                  label={t.integrations.modelInject}
                  injected={status.opencodeModelInjected}
                  path={status.opencodeConfigPath}
                  injectedLabel={t.integrations.modelInjected}
                  notInjectedLabel={t.integrations.modelNotInjected}
                  busy={busy !== null}
                  onToggle={(v) =>
                    void runInject(
                      "opencode-model",
                      () => api.setOpencodeModelInject(v),
                      v ? t.integrations.modelInjectedMsg : t.integrations.modelRemovedMsg
                    )
                  }
                />
                <InjectRow
                  label={t.integrations.mcpInject}
                  injected={status.opencodeMcpInjected}
                  path={status.opencodeConfigPath}
                  injectedLabel={t.integrations.injected}
                  notInjectedLabel={t.integrations.notInjected}
                  busy={busy !== null}
                  onToggle={(v) =>
                    void runInject(
                      "opencode-mcp",
                      () => api.setOpencodeMcpInject(v),
                      v ? t.integrations.injectedMsg : t.integrations.removedMsg
                    )
                  }
                />
              </CardContent>
            </Card>

            {/* WorkBuddy */}
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="flex flex-wrap items-center gap-2 text-base">
                  {t.integrations.clientWorkBuddy}
                  <Badge
                    variant={
                      status.workbuddyModelInjected || status.workbuddyMcpInjected
                        ? "success"
                        : "outline"
                    }
                  >
                    {status.workbuddyModelInjected || status.workbuddyMcpInjected
                      ? t.integrations.injected
                      : t.integrations.notInjected}
                  </Badge>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-0 divide-y divide-neutral-100 p-0">
                <InjectRow
                  label={t.integrations.modelInject}
                  injected={status.workbuddyModelInjected}
                  path={status.workbuddyModelsPath}
                  injectedLabel={t.integrations.modelInjected}
                  notInjectedLabel={t.integrations.modelNotInjected}
                  busy={busy !== null}
                  onToggle={(v) =>
                    void runInject(
                      "workbuddy-model",
                      () => api.setWorkbuddyModelInject(v),
                      v ? t.integrations.modelInjectedMsg : t.integrations.modelRemovedMsg
                    )
                  }
                />
                <InjectRow
                  label={t.integrations.mcpInject}
                  injected={status.workbuddyMcpInjected}
                  path={status.workbuddyMcpPath}
                  injectedLabel={t.integrations.injected}
                  notInjectedLabel={t.integrations.notInjected}
                  busy={busy !== null}
                  onToggle={(v) =>
                    void runInject(
                      "workbuddy-mcp",
                      () => api.setWorkbuddyMcpInject(v),
                      v ? t.integrations.injectedMsg : t.integrations.removedMsg
                    )
                  }
                />
              </CardContent>
            </Card>

            {/* Cursor: MCP inject + BYOK copy */}
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="flex flex-wrap items-center gap-2 text-base">
                  {t.integrations.clientCursor}
                  <Badge variant={status.cursorMcpInjected ? "success" : "outline"}>
                    {status.cursorMcpInjected
                      ? t.integrations.injected
                      : t.integrations.notInjected}
                  </Badge>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-0 divide-y divide-neutral-100 p-0">
                <InjectRow
                  label={t.integrations.mcpInject}
                  injected={status.cursorMcpInjected}
                  path={status.cursorMcpPath}
                  injectedLabel={t.integrations.injected}
                  notInjectedLabel={t.integrations.notInjected}
                  busy={busy !== null}
                  onToggle={(v) =>
                    void runInject(
                      "cursor-mcp",
                      () => api.setCursorMcpInject(v),
                      v ? t.integrations.injectedMsg : t.integrations.removedMsg
                    )
                  }
                />
                <div className="space-y-3 px-4 py-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-medium">{t.integrations.byokTitle}</span>
                    {t.integrations.byokHint ? (
                      <span className="text-xs text-neutral-400">{t.integrations.byokHint}</span>
                    ) : null}
                  </div>
                  <CopyField
                    label={t.integrations.byokBaseUrl}
                    value={status.cursorByokBaseUrl}
                    copyLabel={t.common.copy}
                    showCopyText
                    onCopy={() =>
                      void copyText(t.integrations.byokBaseUrl, status.cursorByokBaseUrl)
                    }
                  />
                  <CopyField
                    label={t.integrations.byokToken}
                    value={status.cursorByokToken}
                    copyLabel={t.common.copy}
                    showCopyText
                    onCopy={() => void copyText(t.integrations.byokToken, status.cursorByokToken)}
                  />
                  <CopyField
                    label={t.integrations.byokModel}
                    value={status.cursorByokModel}
                    copyLabel={t.common.copy}
                    showCopyText
                    onCopy={() => void copyText(t.integrations.byokModel, status.cursorByokModel)}
                  />
                </div>
              </CardContent>
            </Card>
          </div>
        )}

        {SHOW_GROK_BUILD_TAB && tab === "grok-build" && (
          <Card>
            <CardHeader className="py-3">
              <CardTitle className="flex items-center gap-2 text-base">
                <Plug className="h-4 w-4 text-neutral-500" />
                {t.integrations.grokBuild}
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-0 divide-y divide-neutral-100 p-0">
              <div className="flex items-start justify-between gap-4 px-4 py-3">
                <div className="min-w-0 space-y-1">
                  <Badge variant={status.grokBuildInjected ? "success" : "outline"}>
                    {status.grokBuildInjected
                      ? t.integrations.grokBuildInjected
                      : t.integrations.grokBuildNotInjected}
                  </Badge>
                  {t.integrations.grokBuildRestartHint ? (
                    <p className="text-xs text-neutral-500">{t.integrations.grokBuildRestartHint}</p>
                  ) : null}
                  <div className="space-y-0.5 font-mono text-[11px] text-neutral-400">
                    {status.grokBuildConfigPath ? (
                      <p className="truncate">{status.grokBuildConfigPath}</p>
                    ) : null}
                    {status.grokBuildAuthPath ? (
                      <p className="truncate">{status.grokBuildAuthPath}</p>
                    ) : null}
                  </div>
                </div>
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

              <InfoRow
                label={t.integrations.grokBuildProtocol}
                value={status.grokBuildProtocol || "cli-chat-proxy"}
              />
              <InfoRow
                label={t.integrations.grokBuildAccounts}
                value={String(status.grokBuildAccountCount ?? 0)}
              />
              <InfoRow
                label={t.integrations.grokBuildSession}
                value={status.grokBuildSessionEmail || t.integrations.grokBuildSessionEmpty}
                meta={[
                  status.grokBuildSessionTier != null
                    ? `${t.integrations.grokBuildSessionTier} ${status.grokBuildSessionTier}`
                    : null,
                  status.grokBuildSessionReferrer
                    ? `${t.integrations.grokBuildSessionReferrer} ${status.grokBuildSessionReferrer}`
                    : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              />
              <InfoRow
                label={t.integrations.grokBuildBackup}
                value={
                  status.grokBuildRestoreAvailable
                    ? t.integrations.grokBuildBackupReady
                    : t.integrations.grokBuildRestoreUnavailable
                }
              />

              {status.grokBuildSessionWarn ? (
                <div className="px-4 py-3">
                  <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-800">
                    <div className="font-medium">{t.integrations.grokBuildSessionWarnTitle}</div>
                    <p className="mt-1 whitespace-pre-wrap leading-relaxed text-amber-800/90">
                      {status.grokBuildSessionWarn}
                    </p>
                  </div>
                </div>
              ) : null}

              <div className="flex flex-wrap gap-2 px-4 py-3">
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

function InjectRow({
  label,
  injected,
  path,
  injectedLabel,
  notInjectedLabel,
  busy,
  onToggle,
}: {
  label: string;
  injected: boolean;
  path: string;
  injectedLabel: string;
  notInjectedLabel: string;
  busy: boolean;
  onToggle: (v: boolean) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-4 px-4 py-3">
      <div className="min-w-0 space-y-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium">{label}</span>
          <Badge variant={injected ? "success" : "outline"}>
            {injected ? injectedLabel : notInjectedLabel}
          </Badge>
        </div>
        <p className="truncate font-mono text-[11px] text-neutral-400">{path}</p>
      </div>
      <Switch checked={injected} disabled={busy} onCheckedChange={onToggle} />
    </div>
  );
}

/** Compact label / value row used by Grok Build status list. */
function InfoRow({
  label,
  value,
  meta,
}: {
  label: string;
  value: string;
  meta?: string;
}) {
  return (
    <div className="flex items-start justify-between gap-4 px-4 py-2.5">
      <span className="shrink-0 text-sm text-neutral-500">{label}</span>
      <div className="min-w-0 text-right">
        <div className="truncate text-sm font-medium text-neutral-800">{value}</div>
        {meta ? <div className="mt-0.5 truncate text-[11px] text-neutral-400">{meta}</div> : null}
      </div>
    </div>
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
  compact,
}: {
  client: ClientSnippet;
  copyLabel: string;
  onCopy: (e: MouseEvent) => void;
  onOpen: () => void;
  compact?: boolean;
}) {
  return (
    <Card
      className={cn(
        "cursor-pointer transition hover:border-neutral-400 hover:shadow-sm",
        compact && "shadow-none"
      )}
      onClick={onOpen}
    >
      <CardContent
        className={cn("flex items-center gap-3", compact ? "px-3 py-2.5" : "p-4")}
      >
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold">{client.title}</div>
          {compact ? (
            <div className="truncate text-[11px] text-neutral-400">{client.format}</div>
          ) : null}
        </div>
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
