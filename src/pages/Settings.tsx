import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Copy, Download, Eye, EyeOff, Upload } from "lucide-react";
import { disable as disableAutostart, enable as enableAutostart } from "@tauri-apps/plugin-autostart";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  api,
  type Account,
  type AppConfig,
  type AppIconStyle,
  type ModelOptions,
} from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Tabs } from "@/components/ui/tabs";
import { useToast } from "@/components/ui/toast";
import { useI18n } from "@/i18n/context";
import type { LocalePreference } from "@/i18n";
import iconDark from "@/assets/app-icons/dark.png";
import iconLight from "@/assets/app-icons/light.png";
import { useBrand } from "@/components/brand-context";
import { PageBody, PageHeader, PageShell } from "@/components/page-shell";
import { PageLoading } from "@/components/page-loading";

type SettingsTab = "general" | "network" | "proxy" | "models" | "backup";

type BackupSections = {
  basic: boolean;
  token: boolean;
  mappings: boolean;
  accounts: boolean;
};

type BackupFile = {
  format: "grok-go-export";
  version: 1;
  exportedAt: string;
  basic?: Partial<AppConfig>;
  localToken?: string;
  modelMappings?: Record<string, string>;
  accounts?: Account[];
};

const EXPORT_FORMAT = "grok-go-export" as const;

const RESTART_KEYS: (keyof AppConfig)[] = ["preferredPort", "lanEnabled", "bindHost"];

function needsRestart(prev: AppConfig, next: AppConfig): boolean {
  return RESTART_KEYS.some((k) => prev[k] !== next[k]);
}

function ensureOption(list: string[] | undefined, current: string): string[] {
  const base = list?.length ? [...list] : current ? [current] : [];
  if (current && !base.includes(current)) base.unshift(current);
  return base;
}

const defaultSections = (): BackupSections => ({
  basic: true,
  token: false,
  mappings: true,
  accounts: false,
});

export function SettingsPage() {
  const { t, preference, setPreference } = useI18n();
  const { setAppIcon: setBrandIcon } = useBrand();
  const { toast } = useToast();

  const [tab, setTab] = useState<SettingsTab>("general");
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [models, setModels] = useState<ModelOptions | null>(null);
  const [error, setError] = useState("");
  const [showToken, setShowToken] = useState(false);
  const [restartOpen, setRestartOpen] = useState(false);
  const [saving, setSaving] = useState(false);

  const [exportSections, setExportSections] = useState<BackupSections>(defaultSections);
  const [importSections, setImportSections] = useState<BackupSections>(defaultSections);
  const [pendingImport, setPendingImport] = useState<BackupFile | null>(null);
  const [importConfirmOpen, setImportConfirmOpen] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<number | null>(null);
  const configRef = useRef<AppConfig | null>(null);

  useEffect(() => {
    configRef.current = config;
  }, [config]);

  useEffect(() => {
    api
      .getConfig()
      .then((cfg) => {
        setConfig(cfg);
        setBrandIcon(cfg.appIcon ?? "dark");
      })
      .catch((e) => setError(String(e)));
    api.listModelOptions().then(setModels).catch(() => setModels(null));
  }, [setBrandIcon]);

  const persist = useCallback(
    async (next: AppConfig, opts?: { toastMsg?: string; forceRestartPrompt?: boolean }) => {
      const prev = configRef.current;
      setSaving(true);
      try {
        const saved = await api.updateConfig(next);
        setConfig(saved);
        setBrandIcon(saved.appIcon ?? "dark");
        const restart = prev ? needsRestart(prev, saved) : !!opts?.forceRestartPrompt;
        if (opts?.toastMsg) {
          toast(opts.toastMsg, restart ? "warning" : "success");
        } else if (restart) {
          toast(t.settings.toastSavedRestart, "warning");
          setRestartOpen(true);
        } else {
          toast(t.settings.toastSavedApplied, "success");
        }
        if (restart && !opts?.toastMsg) {
          // already opened dialog above
        } else if (restart && opts?.forceRestartPrompt) {
          setRestartOpen(true);
        }
        return { saved, restart } as const;
      } catch (e) {
        toast(`${t.settings.toastError}：${e}`, "error");
        throw e;
      } finally {
        setSaving(false);
      }
    },
    [setBrandIcon, t, toast]
  );

  /** Immediate save for toggles / selects. */
  const applyPatch = useCallback(
    async (patch: Partial<AppConfig>) => {
      if (!configRef.current) return;
      const next = { ...configRef.current, ...patch };
      setConfig(next);
      await persist(next);
    },
    [persist]
  );

  /** Debounced save for text/number fields. */
  const schedulePatch = useCallback(
    (patch: Partial<AppConfig>) => {
      if (!configRef.current) return;
      const next = { ...configRef.current, ...patch };
      setConfig(next);
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
      debounceRef.current = window.setTimeout(() => {
        void persist(next);
      }, 450);
    },
    [persist]
  );

  useEffect(() => {
    return () => {
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
    };
  }, []);

  const tabs = useMemo(
    () =>
      [
        { id: "general" as const, label: t.settings.tabGeneral },
        { id: "network" as const, label: t.settings.tabNetwork },
        { id: "proxy" as const, label: t.settings.tabProxy },
        { id: "models" as const, label: t.settings.tabModels },
        { id: "backup" as const, label: t.settings.tabBackup },
      ] satisfies { id: SettingsTab; label: string }[],
    [t]
  );

  const languageOptions: { value: LocalePreference; label: string }[] = [
    { value: "system", label: t.settings.languageSystem },
    { value: "zh-CN", label: t.settings.languageZhCN },
    { value: "en", label: t.settings.languageEn },
  ];

  const iconOptions: { value: AppIconStyle; label: string; desc: string; src: string }[] = [
    {
      value: "dark",
      label: t.settings.appIconDark,
      desc: t.settings.appIconDarkDesc,
      src: iconDark,
    },
    {
      value: "light",
      label: t.settings.appIconLight,
      desc: t.settings.appIconLightDesc,
      src: iconLight,
    },
  ];

  async function applyIcon(style: AppIconStyle) {
    try {
      const saved = await api.setAppIcon(style);
      setConfig(saved);
      setBrandIcon(style);
      toast(t.settings.appIconApplied, "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function onLaunchOnStartup(v: boolean) {
    if (!config) return;
    try {
      if (v) await enableAutostart();
      else await disableAutostart();
      await applyPatch({ launchOnStartup: v });
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function doExport() {
    if (!config) return;
    if (!exportSections.basic && !exportSections.token && !exportSections.mappings && !exportSections.accounts) {
      toast(t.settings.exportEmpty, "warning");
      return;
    }
    try {
      const accounts = exportSections.accounts ? await api.getAccounts() : [];
      const payload: BackupFile = {
        format: EXPORT_FORMAT,
        version: 1,
        exportedAt: new Date().toISOString(),
      };
      if (exportSections.basic) {
        const {
          localToken: _t,
          modelMappings: _m,
          ...rest
        } = config;
        payload.basic = rest;
      }
      if (exportSections.token) payload.localToken = config.localToken;
      if (exportSections.mappings) payload.modelMappings = { ...config.modelMappings };
      if (exportSections.accounts) payload.accounts = accounts;

      const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `grok-go-backup-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
      toast(t.settings.exportSuccess, "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  function onPickImportFile(file: File) {
    const reader = new FileReader();
    reader.onload = () => {
      try {
        const raw = JSON.parse(String(reader.result)) as BackupFile;
        if (raw?.format !== EXPORT_FORMAT || raw.version !== 1) {
          toast(t.settings.importInvalid, "error");
          return;
        }
        setPendingImport(raw);
        setImportConfirmOpen(true);
      } catch {
        toast(t.settings.importInvalid, "error");
      }
    };
    reader.readAsText(file);
  }

  async function confirmImport() {
    if (!pendingImport || !config) return;
    const has =
      (importSections.basic && pendingImport.basic) ||
      (importSections.token && pendingImport.localToken != null) ||
      (importSections.mappings && pendingImport.modelMappings) ||
      (importSections.accounts && pendingImport.accounts);
    if (!has) {
      toast(t.settings.importNoSection, "warning");
      setImportConfirmOpen(false);
      setPendingImport(null);
      return;
    }

    try {
      let next: AppConfig = { ...config };
      let restart = false;

      if (importSections.basic && pendingImport.basic) {
        const b = pendingImport.basic;
        const candidate: AppConfig = {
          ...next,
          preferredPort: b.preferredPort ?? next.preferredPort,
          lanEnabled: b.lanEnabled ?? next.lanEnabled,
          bindHost: b.bindHost ?? next.bindHost,
          requireToken: b.requireToken ?? next.requireToken,
          defaultModel: b.defaultModel ?? next.defaultModel,
          defaultImageModel: b.defaultImageModel ?? next.defaultImageModel,
          defaultVideoModel: b.defaultVideoModel ?? next.defaultVideoModel,
          routingStrategy: b.routingStrategy ?? next.routingStrategy,
          autoInjectCodexMcp: b.autoInjectCodexMcp ?? next.autoInjectCodexMcp,
          launchOnStartup: b.launchOnStartup ?? next.launchOnStartup,
          minimizeToTray: b.minimizeToTray ?? next.minimizeToTray,
          xaiClientId: b.xaiClientId?.trim() || next.xaiClientId,
          xaiBaseUrl: b.xaiBaseUrl ?? next.xaiBaseUrl,
          oauthRedirectPort: b.oauthRedirectPort ?? next.oauthRedirectPort,
          httpProxyEnabled: b.httpProxyEnabled ?? next.httpProxyEnabled,
          httpProxyUrl: b.httpProxyUrl ?? next.httpProxyUrl,
          appIcon: b.appIcon ?? next.appIcon,
          // keep actualPort from live process unless file has it
          actualPort: b.actualPort ?? next.actualPort,
        };
        restart = needsRestart(next, candidate);
        next = candidate;
      }
      if (importSections.token && pendingImport.localToken != null) {
        next = { ...next, localToken: pendingImport.localToken };
      }
      if (importSections.mappings && pendingImport.modelMappings) {
        next = { ...next, modelMappings: { ...pendingImport.modelMappings } };
      }

      const saved = await api.updateConfig(next);
      setConfig(saved);
      setBrandIcon(saved.appIcon ?? "dark");

      if (importSections.basic && pendingImport.basic?.launchOnStartup != null) {
        try {
          if (saved.launchOnStartup) await enableAutostart();
          else await disableAutostart();
        } catch {
          /* ignore autostart errors on import */
        }
      }
      if (importSections.basic && pendingImport.basic?.appIcon) {
        try {
          await api.setAppIcon(saved.appIcon);
        } catch {
          /* already in config */
        }
      }

      if (importSections.accounts && pendingImport.accounts) {
        await api.replaceAccounts(pendingImport.accounts);
      }

      setImportConfirmOpen(false);
      setPendingImport(null);
      if (restart) {
        toast(t.settings.importPartialRestart, "warning");
        setRestartOpen(true);
      } else {
        toast(t.settings.importSuccess, "success");
      }
    } catch (e) {
      toast(String(e), "error");
    }
  }

  if (!config && !error) return <PageLoading />;

  if (!config) {
    return (
      <div className="space-y-3">
        <div className="text-sm text-red-600">
          {t.common.loadFailed}
          {error ? `：${error}` : ""}
        </div>
        <Button
          variant="outline"
          onClick={() => {
            setError("");
            api.getConfig().then(setConfig).catch((e) => setError(String(e)));
          }}
        >
          {t.common.retry}
        </Button>
      </div>
    );
  }

  return (
    <PageShell>
      <PageHeader className="flex-col items-stretch">
        <div className="space-y-1">
          <h1 className="text-xl font-semibold tracking-tight">{t.settings.title}</h1>
          {t.settings.subtitle ? (
            <p className="text-sm text-neutral-500">{t.settings.subtitle}</p>
          ) : null}
        </div>
      </PageHeader>

      <div className="shrink-0">
        <Tabs items={tabs} value={tab} onChange={setTab} />
      </div>

      <PageBody className="space-y-4 pb-2">
        {tab === "general" && (
          <>
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="text-base">{t.settings.language}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {t.settings.languageDesc ? (
                  <p className="text-sm text-neutral-500">{t.settings.languageDesc}</p>
                ) : null}
                <div className="flex flex-wrap gap-2">
                  {languageOptions.map((opt) => (
                    <Button
                      key={opt.value}
                      size="sm"
                      variant={preference === opt.value ? "default" : "outline"}
                      onClick={() => {
                        setPreference(opt.value);
                        toast(t.settings.toastSavedApplied, "success");
                      }}
                    >
                      {opt.label}
                    </Button>
                  ))}
                </div>
              </CardContent>
            </Card>

            {/* Windows tray uses a fixed black-bg white-logo asset; hide style switch there. */}
            {!(
              typeof navigator !== "undefined" &&
              /windows/i.test(navigator.userAgent || navigator.platform || "")
            ) && (
              <Card>
                <CardHeader className="py-3">
                  <CardTitle className="text-base">{t.settings.appIcon}</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3">
                  {t.settings.appIconDesc ? (
                    <p className="text-sm text-neutral-500">{t.settings.appIconDesc}</p>
                  ) : null}
                  <div className="grid gap-3 sm:grid-cols-2">
                    {iconOptions.map((opt) => {
                      const selected = (config.appIcon ?? "dark") === opt.value;
                      return (
                        <button
                          key={opt.value}
                          type="button"
                          onClick={() => applyIcon(opt.value)}
                          className={`flex items-center gap-3 rounded-xl border p-3 text-left transition ${
                            selected
                              ? "border-neutral-900 bg-neutral-50 ring-2 ring-neutral-900"
                              : "border-neutral-200 hover:border-neutral-400"
                          }`}
                        >
                          <img
                            src={opt.src}
                            alt={opt.label}
                            className="h-14 w-14 shrink-0 rounded-xl border border-neutral-200 object-cover shadow-sm"
                          />
                          <div className="min-w-0">
                            <div className="text-sm font-medium">{opt.label}</div>
                            {opt.desc ? (
                              <div className="text-xs text-neutral-500">{opt.desc}</div>
                            ) : null}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                </CardContent>
              </Card>
            )}

            <Card>
              <CardHeader className="py-3">
                <CardTitle className="text-base">{t.settings.behavior}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                <div className="flex items-center justify-between gap-4 rounded-md border border-neutral-200 px-3 py-2">
                  <div className="min-w-0">
                    <div className="text-sm font-medium">{t.settings.minimizeToTray}</div>
                    {t.settings.minimizeToTrayDesc ? (
                      <div className="text-xs text-neutral-500">{t.settings.minimizeToTrayDesc}</div>
                    ) : null}
                  </div>
                  <Switch
                    checked={config.minimizeToTray}
                    onCheckedChange={(v) => void applyPatch({ minimizeToTray: v })}
                  />
                </div>
                <div className="flex items-center justify-between gap-4 rounded-md border border-neutral-200 px-3 py-2">
                  <div className="min-w-0">
                    <div className="text-sm font-medium">{t.settings.launchOnStartup}</div>
                    {t.settings.launchOnStartupDesc ? (
                      <div className="text-xs text-neutral-500">{t.settings.launchOnStartupDesc}</div>
                    ) : null}
                  </div>
                  <Switch
                    checked={config.launchOnStartup}
                    onCheckedChange={(v) => void onLaunchOnStartup(v)}
                  />
                </div>
              </CardContent>
            </Card>
          </>
        )}

        {tab === "network" && (
          <Card>
            <CardHeader className="py-3">
              <CardTitle className="text-base">{t.settings.network}</CardTitle>
            </CardHeader>
            <CardContent className="grid gap-4 md:grid-cols-2">
              {t.settings.networkDesc ? (
                <p className="text-sm text-neutral-500 md:col-span-2">{t.settings.networkDesc}</p>
              ) : null}
              <div>
                <Label>{t.settings.preferredPort}</Label>
                <Input
                  type="number"
                  value={config.preferredPort}
                  onChange={(e) =>
                    schedulePatch({ preferredPort: Number(e.target.value) || 8787 })
                  }
                />
              </div>
              <div>
                <Label>{t.settings.actualPort}</Label>
                <Input value={config.actualPort} readOnly />
              </div>
              <div className="flex items-center justify-between rounded-md border border-neutral-200 px-3 py-2 md:col-span-2">
                <div>
                  <div className="text-sm font-medium">{t.settings.lanAccess}</div>
                  {t.settings.lanAccessDesc ? (
                    <div className="text-xs text-neutral-500">{t.settings.lanAccessDesc}</div>
                  ) : null}
                </div>
                <Switch
                  checked={config.lanEnabled}
                  onCheckedChange={(v) => void applyPatch({ lanEnabled: v })}
                />
              </div>
              <div className="flex items-center justify-between rounded-md border border-neutral-200 px-3 py-2 md:col-span-2">
                <div>
                  <div className="text-sm font-medium">{t.settings.requireToken}</div>
                  {t.settings.requireTokenDesc ? (
                    <div className="text-xs text-neutral-500">{t.settings.requireTokenDesc}</div>
                  ) : null}
                </div>
                <Switch
                  checked={config.requireToken}
                  onCheckedChange={(v) => void applyPatch({ requireToken: v })}
                />
              </div>

              <div className="md:col-span-2">
                <Label>{t.settings.localToken}</Label>
                <div className="mt-1 flex flex-col gap-2 sm:flex-row sm:items-center">
                  <div className="relative min-w-0 flex-1">
                    <Input
                      className="pr-20 font-mono text-xs"
                      type={showToken ? "text" : "password"}
                      value={config.localToken}
                      readOnly
                      autoComplete="off"
                    />
                    <div className="absolute inset-y-0 right-1 flex items-center gap-0.5">
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-7 w-7"
                        title={showToken ? t.settings.hideToken : t.settings.showToken}
                        onClick={() => setShowToken((v) => !v)}
                      >
                        {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                      </Button>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-7 w-7"
                        title={t.settings.copyToken}
                        onClick={async () => {
                          try {
                            await navigator.clipboard.writeText(config.localToken);
                            toast(t.settings.tokenCopied, "success");
                          } catch {
                            toast(t.settings.tokenCopied, "error");
                          }
                        }}
                      >
                        <Copy className="h-4 w-4" />
                      </Button>
                    </div>
                  </div>
                  <Button
                    variant="outline"
                    onClick={async () => {
                      try {
                        const next = await api.rotateToken();
                        setConfig(next);
                        setShowToken(false);
                        toast(t.settings.tokenRotated, "success");
                      } catch (e) {
                        toast(String(e), "error");
                      }
                    }}
                  >
                    {t.settings.rotate}
                  </Button>
                </div>
              </div>

              <div className="md:col-span-2">
                <Button
                  variant="outline"
                  disabled={saving}
                  onClick={() =>
                    api
                      .startServer()
                      .then(() => toast(t.settings.gatewayEnsured, "success"))
                      .catch((e) => toast(String(e), "error"))
                  }
                >
                  {t.settings.ensureGateway}
                </Button>
              </div>
            </CardContent>
          </Card>
        )}

        {tab === "proxy" && (
          <Card>
            <CardHeader className="py-3">
              <CardTitle className="text-base">{t.settings.httpProxy}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {t.settings.httpProxyDesc ? (
                <p className="text-sm text-neutral-500">{t.settings.httpProxyDesc}</p>
              ) : null}
              <div className="flex items-center justify-between rounded-md border border-neutral-200 px-3 py-2">
                <div className="text-sm font-medium">{t.settings.httpProxyEnabled}</div>
                <Switch
                  checked={config.httpProxyEnabled ?? false}
                  onCheckedChange={(v) => {
                    if (v && !(config.httpProxyUrl ?? "").trim()) {
                      void applyPatch({
                        httpProxyEnabled: true,
                        httpProxyUrl: "http://127.0.0.1:7890",
                      });
                    } else {
                      void applyPatch({ httpProxyEnabled: v });
                    }
                  }}
                />
              </div>
              <div>
                <Label>{t.settings.httpProxyUrl}</Label>
                <Input
                  placeholder={t.settings.httpProxyUrlPlaceholder}
                  value={config.httpProxyUrl ?? ""}
                  disabled={!config.httpProxyEnabled}
                  onChange={(e) => schedulePatch({ httpProxyUrl: e.target.value })}
                />
              </div>
            </CardContent>
          </Card>
        )}

        {tab === "models" && (
          <div className="space-y-4">
            <Card>
              <CardHeader className="py-3">
                <CardTitle className="text-base">{t.settings.models}</CardTitle>
              </CardHeader>
              <CardContent className="grid gap-3 md:grid-cols-3">
                {t.settings.modelsDesc ? (
                  <p className="text-sm text-neutral-500 md:col-span-3">{t.settings.modelsDesc}</p>
                ) : null}
                <div>
                  <Label>{t.settings.defaultText}</Label>
                  <Select
                    value={config.defaultModel}
                    onChange={(e) => void applyPatch({ defaultModel: e.target.value })}
                  >
                    {ensureOption(models?.grokText, config.defaultModel).map((id) => (
                      <option key={id} value={id}>
                        {id}
                      </option>
                    ))}
                  </Select>
                </div>
                <div>
                  <Label>{t.settings.defaultImage}</Label>
                  <Select
                    value={config.defaultImageModel}
                    onChange={(e) => void applyPatch({ defaultImageModel: e.target.value })}
                  >
                    {ensureOption(models?.grokImage, config.defaultImageModel).map((id) => (
                      <option key={id} value={id}>
                        {id}
                      </option>
                    ))}
                  </Select>
                </div>
                <div>
                  <Label>{t.settings.defaultVideo}</Label>
                  <Select
                    value={config.defaultVideoModel}
                    onChange={(e) => void applyPatch({ defaultVideoModel: e.target.value })}
                  >
                    {ensureOption(models?.grokVideo, config.defaultVideoModel).map((id) => (
                      <option key={id} value={id}>
                        {id}
                      </option>
                    ))}
                  </Select>
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="py-3">
                <CardTitle className="text-base">{t.settings.routing}</CardTitle>
              </CardHeader>
              <CardContent className="grid gap-3 md:grid-cols-2">
                {t.settings.routingDesc ? (
                  <p className="text-sm text-neutral-500 md:col-span-2">{t.settings.routingDesc}</p>
                ) : null}
                <div className="md:col-span-2">
                  <Label>{t.settings.routingStrategy}</Label>
                  <Select
                    value={config.routingStrategy}
                    onChange={(e) =>
                      void applyPatch({
                        routingStrategy: e.target.value as AppConfig["routingStrategy"],
                      })
                    }
                  >
                    <option value="weighted-round-robin">{t.settings.routingWrr}</option>
                    <option value="least-recently-used">{t.settings.routingLru}</option>
                    <option value="lowest-error-rate">{t.settings.routingLowestError}</option>
                    <option value="fill-first">{t.settings.routingFillFirst}</option>
                  </Select>
                </div>
                <div className="flex items-center justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2">
                  <div className="min-w-0">
                    <div className="text-sm font-medium">{t.settings.sessionAffinity}</div>
                    {t.settings.sessionAffinityDesc ? (
                      <div className="text-xs text-neutral-500">{t.settings.sessionAffinityDesc}</div>
                    ) : null}
                  </div>
                  <Switch
                    checked={config.sessionAffinity !== false}
                    onCheckedChange={(v) => void applyPatch({ sessionAffinity: v })}
                  />
                </div>
                <div className="flex items-center justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2">
                  <div className="min-w-0">
                    <div className="text-sm font-medium">{t.settings.quotaAwareRouting}</div>
                    {t.settings.quotaAwareRoutingDesc ? (
                      <div className="text-xs text-neutral-500">{t.settings.quotaAwareRoutingDesc}</div>
                    ) : null}
                  </div>
                  <Switch
                    checked={config.quotaAwareRouting !== false}
                    onCheckedChange={(v) => void applyPatch({ quotaAwareRouting: v })}
                  />
                </div>
                <div className="flex items-center justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2">
                  <div className="min-w-0">
                    <div className="text-sm font-medium">{t.settings.preferSoonestReset}</div>
                    {t.settings.preferSoonestResetDesc ? (
                      <div className="text-xs text-neutral-500">{t.settings.preferSoonestResetDesc}</div>
                    ) : null}
                  </div>
                  <Switch
                    checked={Boolean(config.preferSoonestReset)}
                    onCheckedChange={(v) => void applyPatch({ preferSoonestReset: v })}
                  />
                </div>
                <div>
                  <Label>{t.settings.accountMaxConcurrency}</Label>
                  <Input
                    type="number"
                    min={0}
                    value={config.accountMaxConcurrency ?? 6}
                    onChange={(e) =>
                      schedulePatch({
                        accountMaxConcurrency: Math.max(0, Number(e.target.value) || 0),
                      })
                    }
                  />
                  {t.settings.accountMaxConcurrencyDesc ? (
                    <p className="mt-1 text-xs text-neutral-500">{t.settings.accountMaxConcurrencyDesc}</p>
                  ) : null}
                </div>
                <div className="flex items-center justify-between gap-3 rounded-md border border-neutral-200 px-3 py-2 md:col-span-2">
                  <div className="min-w-0">
                    <div className="text-sm font-medium">
                      {t.settings.experimentalImpersonateGrokBuild}
                    </div>
                    {t.settings.experimentalImpersonateGrokBuildDesc ? (
                      <div className="text-xs text-neutral-500">
                        {t.settings.experimentalImpersonateGrokBuildDesc}
                      </div>
                    ) : null}
                  </div>
                  <Switch
                    checked={Boolean(config.experimentalImpersonateGrokBuild)}
                    onCheckedChange={(v) =>
                      void applyPatch({ experimentalImpersonateGrokBuild: v })
                    }
                  />
                </div>
                <div className="md:col-span-2">
                  <Label className="text-sm font-medium">
                    {t.settings.anthropicThinkingMode}
                  </Label>
                  <select
                    className="mt-1 w-full rounded-md border border-neutral-200 bg-white px-3 py-2 text-sm"
                    value={config.anthropicThinkingMode ?? "hide"}
                    onChange={(e) =>
                      void applyPatch({ anthropicThinkingMode: e.target.value })
                    }
                  >
                    <option value="hide">{t.settings.anthropicThinkingHide}</option>
                    <option value="passthrough">
                      {t.settings.anthropicThinkingPassthrough}
                    </option>
                    <option value="summary">{t.settings.anthropicThinkingSummary}</option>
                  </select>
                  {t.settings.anthropicThinkingModeHint ? (
                    <p className="mt-1 text-xs text-neutral-500">
                      {t.settings.anthropicThinkingModeHint}
                    </p>
                  ) : null}
                </div>
              </CardContent>
            </Card>
          </div>
        )}

        {tab === "backup" && (
          <div className="grid gap-4 lg:grid-cols-2">
            <BackupSectionCard
              title={t.settings.exportTitle}
              sections={exportSections}
              setSections={setExportSections}
              t={t}
              action={
                <Button onClick={() => void doExport()}>
                  <Download className="h-4 w-4" />
                  {t.settings.exportAction}
                </Button>
              }
            />
            <BackupSectionCard
              title={t.settings.importTitle}
              sections={importSections}
              setSections={setImportSections}
              t={t}
              action={
                <>
                  <input
                    ref={fileInputRef}
                    type="file"
                    accept="application/json,.json"
                    className="hidden"
                    onChange={(e) => {
                      const f = e.target.files?.[0];
                      if (f) onPickImportFile(f);
                      e.target.value = "";
                    }}
                  />
                  <Button variant="outline" onClick={() => fileInputRef.current?.click()}>
                    <Upload className="h-4 w-4" />
                    {t.settings.importAction}
                  </Button>
                </>
              }
            />
            {t.settings.backupDesc ? (
              <p className="text-sm text-neutral-500 lg:col-span-2">{t.settings.backupDesc}</p>
            ) : null}
          </div>
        )}

      <ConfirmDialog
        open={restartOpen}
        title={t.settings.restartTitle}
        description={t.settings.restartDesc}
        cancelLabel={t.settings.restartLater}
        confirmLabel={t.settings.restartNow}
        onCancel={() => setRestartOpen(false)}
        onConfirm={() => {
          void relaunch().catch((e) => toast(String(e), "error"));
        }}
      />

      <ConfirmDialog
        open={importConfirmOpen}
        title={t.settings.importConfirmTitle}
        description={t.settings.importConfirmDesc}
        cancelLabel={t.common.cancel}
        confirmLabel={t.common.confirm}
        onCancel={() => {
          setImportConfirmOpen(false);
          setPendingImport(null);
        }}
        onConfirm={() => void confirmImport()}
      />
      </PageBody>
    </PageShell>
  );
}

function BackupSectionCard({
  title,
  sections,
  setSections,
  t,
  action,
}: {
  title: string;
  sections: BackupSections;
  setSections: (s: BackupSections) => void;
  t: ReturnType<typeof useI18n>["t"];
  action: ReactNode;
}) {
  const rows: { key: keyof BackupSections; label: string; desc: string }[] = [
    { key: "basic", label: t.settings.sectionBasic, desc: t.settings.sectionBasicDesc },
    { key: "token", label: t.settings.sectionToken, desc: t.settings.sectionTokenDesc },
    { key: "mappings", label: t.settings.sectionMappings, desc: t.settings.sectionMappingsDesc },
    { key: "accounts", label: t.settings.sectionAccounts, desc: t.settings.sectionAccountsDesc },
  ];

  return (
    <Card>
      <CardHeader className="py-3">
        <CardTitle className="text-base">{title}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              setSections({ basic: true, token: true, mappings: true, accounts: true })
            }
          >
            {t.settings.selectAll}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              setSections({ basic: false, token: false, mappings: false, accounts: false })
            }
          >
            {t.settings.selectNone}
          </Button>
        </div>
        <div className="space-y-2">
          {rows.map((row) => (
            <label
              key={row.key}
              className="flex cursor-pointer items-start gap-3 rounded-md border border-neutral-200 px-3 py-2 hover:bg-neutral-50"
            >
              <input
                type="checkbox"
                className="mt-1"
                checked={sections[row.key]}
                onChange={(e) => setSections({ ...sections, [row.key]: e.target.checked })}
              />
              <div className="min-w-0">
                <div className="text-sm font-medium">{row.label}</div>
                {row.desc ? <div className="text-xs text-neutral-500">{row.desc}</div> : null}
              </div>
            </label>
          ))}
        </div>
        <div className="pt-1">{action}</div>
      </CardContent>
    </Card>
  );
}
