import { useEffect, useState } from "react";
import { api, type AppStatus, type HeatmapDay } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Heatmap } from "@/components/heatmap";
import { formatNumber, formatUsd } from "@/lib/utils";
import { useI18n } from "@/i18n/context";
import { PageLoading } from "@/components/page-loading";
import { useToast } from "@/components/ui/toast";
import { Copy, Download, Eye, EyeOff, RefreshCw } from "lucide-react";

const HEATMAP_DAYS = 371;

function maskToken(token: string): string {
  if (!token) return "";
  if (token.length <= 8) return "••••••••";
  return `${token.slice(0, 4)}${"•".repeat(Math.min(token.length - 8, 24))}${token.slice(-4)}`;
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
      const [s, h] = await Promise.all([api.getStatus(), api.getHeatmap(HEATMAP_DAYS)]);
      setStatus(s);
      setHeatmap(h);
      setError("");
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
      await api.importToCcSwitch();
      toast(t.overview.importCcSwitchSuccess, "success");
    } catch (e) {
      toast(`${t.overview.importCcSwitchFailed}：${e}`, "error");
    } finally {
      setImporting(false);
    }
  }

  if (loading && !status) {
    return <PageLoading />;
  }

  if (!status) {
    return (
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
    );
  }

  return (
    <div className="space-y-4 overflow-y-auto">
      <div className="flex items-center justify-between gap-4">
        <h1 className="text-xl font-semibold tracking-tight">{t.overview.title}</h1>
        <Button variant="outline" size="sm" onClick={() => refresh()}>
          <RefreshCw className="h-4 w-4" />
          {t.common.refresh}
        </Button>
      </div>

      <div className="grid gap-3 md:grid-cols-4">
        <Card>
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
        <Card>
          <CardHeader className="pb-2">
            <CardDescription>{t.overview.accounts}</CardDescription>
            <CardTitle className="text-lg">
              {status.healthyAccounts}/{status.accountCount}
            </CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-neutral-500">{t.overview.healthyAccounts}</CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardDescription>{t.overview.todayRequests}</CardDescription>
            <CardTitle className="text-lg">{formatNumber(status.today.totalRequests)}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-neutral-500">
            {formatNumber(status.today.successRequests)} {t.overview.success}
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardDescription>{t.overview.todayTokens}</CardDescription>
            <CardTitle className="text-lg">
              {formatNumber(
                status.today.inputTokens + status.today.outputTokens + status.today.cacheTokens
              )}
            </CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-neutral-500">
            {t.overview.est} {formatUsd(status.today.estimatedCostUsd)}
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
          {(
            [
              [t.overview.baseUrl, status.baseUrl, false],
              [t.overview.mcp, status.mcpUrl, false],
              [t.overview.localToken, status.localToken, true],
            ] as const
          ).map(([label, value, isToken]) => (
            <div key={label} className="rounded-md border border-neutral-200 p-2.5">
              <div className="mb-1 text-xs font-medium text-neutral-500">{label}</div>
              <div className="flex items-center justify-between gap-3">
                <code className="min-w-0 truncate text-sm">
                  {isToken && !showToken ? maskToken(value) : value}
                </code>
                <div className="flex shrink-0 items-center gap-0.5">
                  {isToken ? (
                    <Button
                      size="icon"
                      variant="ghost"
                      title={showToken ? t.settings.hideToken : t.settings.showToken}
                      onClick={() => setShowToken((v) => !v)}
                    >
                      {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </Button>
                  ) : null}
                  <Button
                    size="icon"
                    variant="ghost"
                    title={t.common.copy}
                    onClick={() => copy(value)}
                  >
                    <Copy className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            </div>
          ))}
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
    </div>
  );
}
