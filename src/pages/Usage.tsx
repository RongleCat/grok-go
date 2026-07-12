import { useEffect, useState } from "react";
import { api, type HeatmapDay, type UsageSummary } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Heatmap } from "@/components/heatmap";
import { formatNumber, formatUsd } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { useI18n } from "@/i18n/context";
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
      const [s, h] = await Promise.all([api.getUsageSummary(), api.getHeatmap(371)]);
      setSummary(s);
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
    <div className="space-y-4 overflow-y-auto">
      <div className="flex items-center justify-between gap-4">
        <h1 className="text-xl font-semibold tracking-tight">{t.usage.title}</h1>
        <Button variant="outline" size="sm" onClick={refresh}>
          {t.common.refresh}
        </Button>
      </div>

      <div className="grid gap-3 md:grid-cols-4">
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
    </div>
  );
}
