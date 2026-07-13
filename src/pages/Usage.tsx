import { useEffect, useState } from "react";
import { api, type HeatmapDay, type UsageSummary } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Heatmap } from "@/components/heatmap";
import { formatCacheHitRate, formatNumber, formatUsd } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/context";
import { PageBody, PageHeader, PageShell } from "@/components/page-shell";
import { PageLoading } from "@/components/page-loading";

export function UsagePage() {
  const { t } = useI18n();
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [heatmap, setHeatmap] = useState<HeatmapDay[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);

  async function refresh() {
    setLoading(true);
    try {
      const s = await api.getUsageSummary();
      setSummary(s);
      setError("");
      try {
        const h = await api.getHeatmap(371);
        setHeatmap(h);
      } catch (heatErr) {
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
  }, []);

  if (loading && !summary) {
    return <PageLoading />;
  }

  if (!summary) {
    return (
      <div className="space-y-3">
        <div className="text-sm text-red-600">
          {t.common.loadFailed}
          {error ? `：${error}` : ""}
        </div>
        <Button variant="outline" onClick={refresh}>
          {t.common.retry}
        </Button>
      </div>
    );
  }

  return (
    <PageShell>
      <PageHeader>
        <h1 className="text-xl font-semibold tracking-tight">{t.usage.title}</h1>
        <Button variant="outline" size="sm" onClick={refresh}>
          {t.common.refresh}
        </Button>
      </PageHeader>

      <PageBody className="space-y-4">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-5">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-xl">{formatNumber(summary.totalRequests)}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-neutral-500">{t.usage.requestsToday}</CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-xl">{formatNumber(summary.inputTokens)}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-neutral-500">{t.usage.inputTokens}</CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-xl">{formatNumber(summary.outputTokens)}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-neutral-500">{t.usage.outputTokens}</CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-xl text-emerald-700">
              {formatNumber(summary.cacheTokens)}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-0.5 text-sm text-neutral-500">
            <div>{t.usage.cacheTokens}</div>
            <div
              className="text-xs text-neutral-400"
              title={t.usage.cacheHitRateHint}
            >
              {t.usage.cacheHitRate}{" "}
              <span className="font-medium text-neutral-600">
                {formatCacheHitRate(summary.inputTokens, summary.cacheTokens)}
              </span>
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-xl">{formatUsd(summary.estimatedCostUsd)}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm text-neutral-500">{t.usage.estimatedCost}</CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader className="py-3">
          <CardTitle className="text-base">{t.usage.heatmap}</CardTitle>
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
