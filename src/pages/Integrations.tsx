import { useCallback, useEffect, useMemo, useState, type MouseEvent } from "react";
import { Copy, FileText, Plug, RefreshCw } from "lucide-react";
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
import { PageLoading } from "@/components/page-loading";
import { cn } from "@/lib/utils";

type TabId = "codex" | "mcp" | "clients" | "grok-build";

/** Grok Build API-key endpoint inject — code ready, UI hidden until GA. */
const SHOW_GROK_BUILD_TAB = false;

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
      { id: "mcp", label: t.integrations.tabMcp },
      { id: "clients", label: t.integrations.tabClients },
    ];
    if (SHOW_GROK_BUILD_TAB) {
      list.push({ id: "grok-build", label: t.integrations.tabGrokBuild });
    }
    return list;
  }, [t]);

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
    <div className="space-y-4">
      <div className="flex items-start justify-between gap-3">
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
      </div>

      <Tabs items={tabs} value={tab} onChange={setTab} />

      <div className="space-y-4 pb-2">
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
                      toast(msg || t.integrations.importCcSwitch, "success");
                      await load();
                    } catch (e) {
                      toast(String(e), "error");
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
                      <p className="mt-0.5 text-xs text-neutral-500">{desc}</p>
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
          <Card>
            <CardHeader className="py-3">
              <CardTitle className="flex items-center gap-2 text-base">
                {t.integrations.grokBuild}
                <Badge variant={status.grokBuildInjected ? "success" : "outline"}>
                  {status.grokBuildInjected
                    ? t.integrations.grokBuildInjected
                    : t.integrations.grokBuildNotInjected}
                </Badge>
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                <div className="min-w-0 space-y-1">
                  {t.integrations.grokBuildDesc ? (
                    <p className="text-sm text-neutral-500">{t.integrations.grokBuildDesc}</p>
                  ) : null}
                  <p className="truncate font-mono text-[11px] text-neutral-400">
                    {status.grokBuildConfigPath}
                  </p>
                  {t.integrations.grokBuildEnvHint ? (
                    <p className="text-xs text-neutral-500">{t.integrations.grokBuildEnvHint}</p>
                  ) : null}
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
              {status.grokBuildSnippet ? (
                <div className="flex flex-wrap gap-2">
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
                </div>
              ) : null}
            </CardContent>
          </Card>
        )}
      </div>

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
