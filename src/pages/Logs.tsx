import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, type Account, type RequestLog } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  formatCacheHitRate,
  formatNumber,
  formatTokenCompact,
  formatUsd,
} from "@/lib/utils";
import { useI18n } from "@/i18n/context";

const PAGE_SIZE = 50;
const ROW_HEIGHT = 56;
const OVERSCAN = 8;

/** Dense token cell: total + in/out/cache + hit rate when cache > 0. */
function TokenCell({
  log,
  labels,
}: {
  log: RequestLog;
  labels: {
    tokenIn: string;
    tokenOut: string;
    tokenCache: string;
    tokenCacheHit: string;
  };
}) {
  const input = log.inputTokens || 0;
  const output = log.outputTokens || 0;
  const cache = log.cacheTokens || 0;
  const total = input + output + cache;
  const detail = `${labels.tokenIn} ${formatNumber(input)} · ${labels.tokenOut} ${formatNumber(output)} · ${labels.tokenCache} ${formatNumber(cache)}`;
  const hit = formatCacheHitRate(input, cache);

  return (
    <div className="min-w-0 leading-tight" title={detail}>
      <div className="text-xs font-medium tabular-nums">{formatTokenCompact(total)}</div>
      <div className="text-[10px] tabular-nums text-neutral-400 whitespace-nowrap">
        <span className="text-neutral-500">{labels.tokenIn}</span> {formatTokenCompact(input)}
        <span className="mx-0.5 text-neutral-300">·</span>
        <span className="text-neutral-500">{labels.tokenOut}</span> {formatTokenCompact(output)}
        <span className="mx-0.5 text-neutral-300">·</span>
        <span className={cache > 0 ? "text-emerald-600" : "text-neutral-500"}>
          {labels.tokenCache}
        </span>{" "}
        <span className={cache > 0 ? "text-emerald-700" : undefined}>
          {formatTokenCompact(cache)}
        </span>
        {cache > 0 ? (
          <span className="text-emerald-600/80">
            {" "}
            ({labels.tokenCacheHit} {hit})
          </span>
        ) : null}
      </div>
    </div>
  );
}

function accountLabel(
  accountId: string | null | undefined,
  byId: Map<string, Account>
): string {
  if (!accountId) return "—";
  const acc = byId.get(accountId);
  if (!acc) {
    // Fallback: short id when account was deleted
    return accountId.length > 12 ? `${accountId.slice(0, 8)}…` : accountId;
  }
  return (acc.email || acc.name || accountId).trim() || accountId;
}

export function LogsPage() {
  const { t, locale } = useI18n();
  const [logs, setLogs] = useState<RequestLog[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportH, setViewportH] = useState(400);
  const listRef = useRef<HTMLDivElement>(null);
  const loadingMore = useRef(false);

  const accountById = useMemo(() => {
    const m = new Map<string, Account>();
    for (const a of accounts) m.set(a.id, a);
    return m;
  }, [accounts]);

  const loadPage = useCallback(async (offset: number, replace: boolean) => {
    if (loadingMore.current) return;
    loadingMore.current = true;
    setLoading(true);
    try {
      const page = await api.getRecentLogs(PAGE_SIZE, offset);
      setLogs((prev) => (replace ? page : [...prev, ...page]));
      setHasMore(page.length >= PAGE_SIZE);
      setError("");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
      loadingMore.current = false;
    }
  }, []);

  async function refresh() {
    setHasMore(true);
    try {
      const list = await api.getAccounts();
      setAccounts(list);
    } catch {
      /* non-fatal for logs */
    }
    await loadPage(0, true);
  }

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      setViewportH(entries[0]?.contentRect.height ?? 400);
    });
    ro.observe(el);
    setViewportH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  function onScroll(e: React.UIEvent<HTMLDivElement>) {
    const el = e.currentTarget;
    setScrollTop(el.scrollTop);
    const nearBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - ROW_HEIGHT * 4;
    if (nearBottom && hasMore && !loadingMore.current) {
      loadPage(logs.length, false);
    }
  }

  const total = logs.length;
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const visibleCount = Math.ceil(viewportH / ROW_HEIGHT) + OVERSCAN * 2;
  const end = Math.min(total, start + visibleCount);
  const visible = logs.slice(start, end);
  const padTop = start * ROW_HEIGHT;
  const padBottom = Math.max(0, (total - end) * ROW_HEIGHT);

  // Account/time(+status) | Source+Endpoint | Model | Latency | Token (wide) | Cost
  const gridCols =
    "grid-cols-[minmax(196px,1.35fr)_minmax(100px,1fr)_minmax(88px,0.85fr)_52px_minmax(200px,1.7fr)_60px]";

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex shrink-0 items-center justify-between gap-3">
        <h1 className="text-xl font-semibold tracking-tight">{t.logs.title}</h1>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => void refresh()}>
            {t.common.refresh}
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={async () => {
              await api.clearLogs();
              await refresh();
            }}
          >
            {t.logs.clear}
          </Button>
        </div>
      </div>

      <Card className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <CardHeader className="shrink-0 py-3">
          <CardTitle className="text-base">{t.logs.recent}</CardTitle>
        </CardHeader>
        <CardContent className="flex min-h-0 flex-1 flex-col p-0">
          <div className="shrink-0 border-b border-neutral-200 px-4">
            <div className={`grid ${gridCols} gap-2 py-2 text-xs font-medium text-neutral-500`}>
              <div>{t.logs.accountTime}</div>
              <div>
                <div>{t.logs.source}</div>
                <div className="font-normal text-neutral-400">{t.logs.endpoint}</div>
              </div>
              <div>{t.logs.model}</div>
              <div>{t.logs.latency}</div>
              <div title={`${t.logs.tokenIn} / ${t.logs.tokenOut} / ${t.logs.tokenCache}`}>
                {t.logs.tokens}
              </div>
              <div>{t.logs.cost}</div>
            </div>
          </div>

          <div
            ref={listRef}
            className="min-h-0 flex-1 overflow-y-auto px-4"
            onScroll={onScroll}
          >
            {total === 0 && !loading ? (
              <div className="py-10 text-center text-sm text-neutral-500">{t.logs.empty}</div>
            ) : (
              <div style={{ height: total * ROW_HEIGHT + (hasMore || loading ? 36 : 0) }}>
                <div style={{ height: padTop }} />
                {visible.map((log) => {
                  const hitAccount = accountLabel(log.accountId, accountById);
                  const ok = log.statusCode >= 200 && log.statusCode < 400;
                  const timeStr = new Date(log.createdAt).toLocaleString(
                    locale === "zh-CN" ? "zh-CN" : "en-US",
                    {
                      month: "numeric",
                      day: "numeric",
                      hour: "2-digit",
                      minute: "2-digit",
                      second: "2-digit",
                      hour12: false,
                    }
                  );
                  return (
                    <div
                      key={log.requestId}
                      className={`grid ${gridCols} items-center gap-2 border-b border-neutral-100 text-sm`}
                      style={{ height: ROW_HEIGHT }}
                    >
                      <div className="min-w-0 leading-tight">
                        <div
                          className="truncate text-[11px] font-medium text-neutral-800"
                          title={
                            log.accountId
                              ? `${hitAccount} (${log.accountId})`
                              : t.logs.accountUnknown
                          }
                        >
                          {hitAccount}
                        </div>
                        <div className="mt-0.5 flex min-w-0 items-center gap-1.5">
                          <span
                            className={
                              ok
                                ? "inline-flex shrink-0 items-center rounded px-1 py-px text-[10px] font-semibold tabular-nums leading-none bg-emerald-600 text-white"
                                : "inline-flex shrink-0 items-center rounded px-1 py-px text-[10px] font-semibold tabular-nums leading-none bg-red-600 text-white"
                            }
                            title={`${t.logs.status} ${log.statusCode}`}
                          >
                            {log.statusCode}
                          </span>
                          <span
                            className="min-w-0 truncate text-[10px] tabular-nums text-neutral-500"
                            title={new Date(log.createdAt).toLocaleString(
                              locale === "zh-CN" ? "zh-CN" : "en-US"
                            )}
                          >
                            {timeStr}
                          </span>
                        </div>
                      </div>
                      <div className="min-w-0 leading-tight">
                        <div
                          className="truncate text-[11px] text-neutral-700"
                          title={log.clientSource || undefined}
                        >
                          {log.clientSource || "—"}
                        </div>
                        <div
                          className="truncate font-mono text-[10px] text-neutral-500"
                          title={log.endpoint}
                        >
                          {log.endpoint}
                        </div>
                      </div>
                      <div className="min-w-0 truncate text-xs">
                        <div className="truncate" title={log.resolvedModel || undefined}>
                          {log.resolvedModel || "-"}
                        </div>
                        {log.requestedModel && log.requestedModel !== log.resolvedModel ? (
                          <div
                            className="truncate text-[10px] text-neutral-400"
                            title={`${t.logs.from} ${log.requestedModel}`}
                          >
                            {t.logs.from} {log.requestedModel}
                          </div>
                        ) : null}
                      </div>
                      <div className="text-xs tabular-nums text-neutral-600">
                        {log.latencyMs}ms
                      </div>
                      <TokenCell
                        log={log}
                        labels={{
                          tokenIn: t.logs.tokenIn,
                          tokenOut: t.logs.tokenOut,
                          tokenCache: t.logs.tokenCache,
                          tokenCacheHit: t.logs.tokenCacheHit,
                        }}
                      />
                      <div className="text-xs tabular-nums text-neutral-700">
                        {formatUsd(log.estimatedCostUsd)}
                      </div>
                    </div>
                  );
                })}
                <div style={{ height: padBottom }} />
                {loading ? (
                  <div className="py-2 text-center text-xs text-neutral-400">{t.common.loading}</div>
                ) : null}
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      {error ? <div className="shrink-0 text-sm text-red-600">{error}</div> : null}
    </div>
  );
}
