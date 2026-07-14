import { useEffect, useState } from "react";
import { api, type AppStatus, type HeatmapDay } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Heatmap } from "@/components/heatmap";
import { formatNumber, formatUsd } from "@/lib/utils";
import { useI18n } from "@/i18n/context";
import { PageLoading } from "@/components/page-loading";
import { PageBody, PageHeader, PageShell } from "@/components/page-shell";
import { useToast } from "@/components/ui/toast";
import { Copy, Download, Eye, EyeOff, RefreshCw } from "lucide-react";

const HEATMAP_DAYS = 371;

function maskToken(token: string): string {
  if (!token) return "";
  if (token.length <= 8) return "••••••••";
  return `${token.slice(0, 4)}${"•".repeat(Math.min(token.length - 8, 24))}${token.slice(-4)}`;
}

function TokenStat({
  label,
  value,
  accent,
}: {
  label: string;
  value: number;
  accent?: "default" | "emerald";
}) {
  return (
    <div className="min-w-0 rounded-lg bg-neutral-50 px-3 py-2.5">
      <div className="text-[11px] font-medium uppercase tracking-wide text-neutral-400">
        {label}
      </div>
      <div
        className={
          accent === "emerald"
            ? "mt-0.5 text-base font-semibold tabular-nums text-emerald-700"
            : "mt-0.5 text-base font-semibold tabular-nums text-neutral-900"
        }
      >
        {formatNumber(value)}
      </div>
    </div>
  );
}

export function OverviewPage() {
  const { t } = useI18n();
  const { toast } = useToast();
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [heatmap, setHeatmap] = useState<HeatmapDay[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [showToken, setShowToken] = useState(false);

  async function refresh() {
    try {
      const s = await api.getStatus();
      setStatus(s);
      setError("");
      try {
        const h = await api.getHeatmap(HEATMAP_DAYS);
        setHeatmap(h);
      } catch (heatErr) {
        // Empty DB / first launch should not blank the whole overview.
        console.warn("heatmap load failed", heatErr);
        setHeatmap([]);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    refresh();
    const timer = setInterval(() => {
      refresh().catch(() => undefined);
    }, 4000);
    return () => clearInterval(timer);
  }, []);

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast(t.common.copied, "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }

  async function importCcSwitch() {
    setImporting(true);
    try {
      const msg = await api.importToCcSwitch();
      // Prefer backend humanized message (update vs create, MCP notes, etc.).
      toast(msg?.trim() || t.overview.importCcSwitchSuccess, "success");
    } catch (e) {
      const raw = String(e);
      // Tauri often wraps as `... error message`; surface the useful part.
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
      setImporting(false);
    }
  }

  if (loading && !status) {
    return <PageLoading />;
  }

  if (!status) {
    return (
      <PageShell>
        <div className="space-y-3">
          <div className="text-sm text-red-600">
            {t.common.loadFailed}
            {error ? `：${error}` : ""}
          </div>
          <Button
            variant="outline"
            onClick={() => {
              setLoading(true);
              refresh();
            }}
          >
            {t.common.retry}
          </Button>
        </div>
      </PageShell>
    );
  }

  // input already includes cache hits — do not add cacheTokens again.
  const totalTokens = status.today.inputTokens + status.today.outputTokens;

  return (
    <PageShell>
      <PageHeader>
        <h1 className="text-xl font-semibold tracking-tight">{t.overview.title}</h1>
        <Button variant="outline" size="sm" onClick={() => refresh()}>
          <RefreshCw className="h-4 w-4" />
          {t.common.refresh}
        </Button>
      </PageHeader>

      <PageBody className="space-y-4">
        {/* Metrics row: gateway | accounts | requests | tokens  → flex 1 1 1 3 */}
        <div className="flex flex-col gap-3 lg:flex-row lg:items-stretch">
          <Card className="min-w-0 flex-1">
            <CardHeader className="pb-2">
              <CardDescription>{t.overview.gateway}</CardDescription>
              <CardTitle className="flex items-center gap-2 text-lg">
                {status.running ? t.overview.running : t.overview.stopped}
                <Badge variant={status.running ? "success" : "danger"}>
                  {status.running ? t.overview.online : t.overview.offline}
                </Badge>
              </CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-neutral-500">
              {t.overview.port} {status.actualPort}
              {status.actualPort !== status.preferredPort
                ? ` (${t.overview.preferred} ${status.preferredPort})`
                : ""}
            </CardContent>
          </Card>
          <Card className="min-w-0 flex-1">
            <CardHeader className="pb-2">
              <CardDescription>{t.overview.accounts}</CardDescription>
              <CardTitle className="text-lg">
                {status.healthyAccounts}/{status.accountCount}
              </CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-neutral-500">
              {t.overview.healthyAccounts}
            </CardContent>
          </Card>
          <Card className="min-w-0 flex-1">
            <CardHeader className="pb-2">
              <CardDescription>{t.overview.todayRequests}</CardDescription>
              <CardTitle className="text-lg">
                {formatNumber(status.today.totalRequests)}
              </CardTitle>
            </CardHeader>
            <CardContent className="text-sm text-neutral-500">
              {formatNumber(status.today.successRequests)} {t.overview.success}
            </CardContent>
          </Card>
          <Card className="min-w-0 flex-[3]">
            <CardContent className="flex h-full flex-col justify-center p-4">
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div>
                  <div className="text-xs font-medium text-neutral-500">
                    {t.overview.todayTokens}
                  </div>
                  <div className="mt-0.5 text-xl font-semibold tabular-nums tracking-tight text-neutral-900">
                    {formatNumber(totalTokens)}
                  </div>
                </div>
                <div className="rounded-full bg-neutral-100 px-2.5 py-1 text-xs font-medium tabular-nums text-neutral-600">
                  {t.overview.est} {formatUsd(status.today.estimatedCostUsd)}
                </div>
              </div>
              <div className="mt-2.5 grid grid-cols-3 gap-2">
                <TokenStat label={t.overview.tokenIn} value={status.today.inputTokens} />
                <TokenStat label={t.overview.tokenOut} value={status.today.outputTokens} />
                <TokenStat
                  label={t.overview.tokenCache}
                  value={status.today.cacheTokens}
                  accent="emerald"
                />
              </div>
            </CardContent>
          </Card>
        </div>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between gap-4 space-y-0 py-3">
            <CardTitle className="text-base">{t.overview.endpoints}</CardTitle>
            <Button size="sm" onClick={importCcSwitch} disabled={importing}>
              <Download className="h-4 w-4" />
              {importing ? t.common.loading : t.overview.importCcSwitch}
            </Button>
          </CardHeader>
          <CardContent className="space-y-2">
            {/* API + MCP on one row */}
            <div className="grid gap-2 sm:grid-cols-2">
              {(
                [
                  [t.overview.baseUrl, status.baseUrl],
                  [t.overview.mcp, status.mcpUrl],
                ] as const
              ).map(([label, value]) => (
                <div key={label} className="min-w-0 rounded-md border border-neutral-200 p-2.5">
                  <div className="mb-1 text-xs font-medium text-neutral-500">{label}</div>
                  <div className="flex items-center justify-between gap-2">
                    <code className="min-w-0 truncate text-sm" title={value}>
                      {value}
                    </code>
                    <Button
                      size="icon"
                      variant="ghost"
                      className="shrink-0"
                      title={t.common.copy}
                      onClick={() => copy(value)}
                    >
                      <Copy className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
            {/* Local token — full width row */}
            <div className="rounded-md border border-neutral-200 p-2.5">
              <div className="mb-1 text-xs font-medium text-neutral-500">
                {t.overview.localToken}
              </div>
              <div className="flex items-center justify-between gap-3">
                <code className="min-w-0 truncate text-sm">
                  {showToken ? status.localToken : maskToken(status.localToken)}
                </code>
                <div className="flex shrink-0 items-center gap-0.5">
                  <Button
                    size="icon"
                    variant="ghost"
                    title={showToken ? t.settings.hideToken : t.settings.showToken}
                    onClick={() => setShowToken((v) => !v)}
                  >
                    {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                  </Button>
                  <Button
                    size="icon"
                    variant="ghost"
                    title={t.common.copy}
                    onClick={() => copy(status.localToken)}
                  >
                    <Copy className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </div>
            {status.lanEnabled && status.lanAddress ? (
              <div className="text-sm text-neutral-500">
                {t.overview.lanHost}: {status.lanAddress}
              </div>
            ) : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="py-3">
            <CardTitle className="text-base">{t.overview.activity}</CardTitle>
          </CardHeader>
          <CardContent>
            <Heatmap days={heatmap} metric="requests" />
          </CardContent>
        </Card>

        {error ? <div className="text-sm text-amber-600">{error}</div> : null}
      </PageBody>
    </PageShell>
  );
}
