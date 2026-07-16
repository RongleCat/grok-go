import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Inbox, Settings2 } from "lucide-react";
import { api, type Account, type AppConfig, type LogStoreStats, type RequestLog } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog } from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { PageHeader, PageShell } from "@/components/page-shell";
import {
  formatCacheHitRate,
  formatNumber,
  formatTokenCompact,
  formatUsd,
} from "@/lib/utils";
import { useI18n } from "@/i18n/context";
import { useToast } from "@/components/ui/toast";

const PAGE_SIZE = 50;
/** Cap in-memory / scroll-list rows (newest-first contiguous window). */
const MAX_LOADED = 200;
const ROW_HEIGHT = 56;
const OVERSCAN = 8;
/** Poll newest page while Logs page is open. */
const POLL_MS = 4000;
/** Within this scroll offset, treat as “following latest” (stay at top). */
const STICK_TOP_PX = ROW_HEIGHT;

function formatMs(ms: number | null | undefined): string {
  if (ms == null || Number.isNaN(ms)) return "—";
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  return s < 10 ? `${s.toFixed(1)}s` : `${Math.round(s)}s`;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/** First-token / total, stacked like token cell. */
function LatencyCell({
  log,
  labels,
}: {
  log: RequestLog;
  labels: { ttft: string; total: string };
}) {
  const ft = log.firstTokenMs;
  const total = log.latencyMs;
  const title =
    ft != null
      ? `${labels.ttft} ${ft}ms · ${labels.total} ${total}ms`
      : `${labels.total} ${total}ms`;
  return (
    <div className="min-w-0 leading-tight tabular-nums" title={title}>
      <div className="text-[11px] font-medium text-neutral-800">
        <span className="text-[10px] font-normal text-neutral-400">{labels.ttft}</span>{" "}
        {formatMs(ft ?? null)}
      </div>
      <div className="text-[10px] text-neutral-500">
        <span className="text-neutral-400">{labels.total}</span> {formatMs(total)}
      </div>
    </div>
  );
}

/**
 * Dense token cell: total + in/out/cache + hit rate when cache > 0.
 * `inputTokens` already includes cache hits — total is input+output only.
 */
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
  const total = input + output;
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
    return accountId.length > 12 ? `${accountId.slice(0, 8)}…` : accountId;
  }
  return (acc.email || acc.name || accountId).trim() || accountId;
}

function toDateInputValue(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso.slice(0, 10);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function LogsPage() {
  const { t, locale } = useI18n();
  const { toast } = useToast();
  const [logs, setLogs] = useState<RequestLog[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportH, setViewportH] = useState(400);
  const listRef = useRef<HTMLDivElement>(null);
  const loadingMore = useRef(false);
  const pollInFlight = useRef(false);
  const logsRef = useRef<RequestLog[]>([]);
  /** After prepend poll: null = no adjust; number = absolute scrollTop to apply. */
  const pendingScrollTop = useRef<number | null>(null);
  logsRef.current = logs;

  const [manageOpen, setManageOpen] = useState(false);
  const [stats, setStats] = useState<LogStoreStats | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [retentionDays, setRetentionDays] = useState(30);
  const [maxRows, setMaxRows] = useState(50_000);
  const [rangeFrom, setRangeFrom] = useState("");
  const [rangeTo, setRangeTo] = useState("");
  const [busy, setBusy] = useState(false);

  const accountById = useMemo(() => {
    const m = new Map<string, Account>();
    for (const a of accounts) m.set(a.id, a);
    return m;
  }, [accounts]);

  const resetScroll = useCallback(() => {
    setScrollTop(0);
    if (listRef.current) listRef.current.scrollTop = 0;
  }, []);

  const loadPage = useCallback(async (offset: number, replace: boolean) => {
    if (loadingMore.current) return;
    if (!replace && offset >= MAX_LOADED) {
      setHasMore(false);
      return;
    }
    loadingMore.current = true;
    if (replace) setLoading(true);
    try {
      const limit = replace
        ? PAGE_SIZE
        : Math.min(PAGE_SIZE, Math.max(0, MAX_LOADED - offset));
      if (!replace && limit <= 0) {
        setHasMore(false);
        return;
      }
      const page = await api.getRecentLogs(limit, offset);
      setLogs((prev) => {
        if (replace) return page.slice(0, MAX_LOADED);
        const seen = new Set(prev.map((l) => l.requestId));
        const extra = page.filter((l) => !seen.has(l.requestId));
        return [...prev, ...extra].slice(0, MAX_LOADED);
      });
      setHasMore(page.length >= limit && offset + page.length < MAX_LOADED);
      setError("");
      if (replace) {
        setScrollTop(0);
        requestAnimationFrame(() => {
          if (listRef.current) listRef.current.scrollTop = 0;
        });
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
      loadingMore.current = false;
    }
  }, []);

  /**
   * Silent poll of the newest page. Prepends new rows without jumping the
   * viewport when the user has scrolled down (scrollTop compensation).
   * When near top, stay pinned so latest traffic is visible.
   */
  const pollLatest = useCallback(async () => {
    if (pollInFlight.current || loadingMore.current) return;
    if (typeof document !== "undefined" && document.visibilityState !== "visible") {
      return;
    }
    pollInFlight.current = true;
    try {
      const page = await api.getRecentLogs(PAGE_SIZE, 0);
      const prev = logsRef.current;
      const prevById = new Map(prev.map((l) => [l.requestId, l]));
      let added = 0;
      for (const row of page) {
        if (!prevById.has(row.requestId)) added += 1;
      }
      const pageIds = new Set(page.map((l) => l.requestId));
      const tail = prev.filter((l) => !pageIds.has(l.requestId));
      let next = [...page, ...tail];
      let truncated = false;
      if (next.length > MAX_LOADED) {
        next = next.slice(0, MAX_LOADED);
        truncated = true;
      }

      // Skip setState if nothing meaningful changed (same ids + key fields).
      let changed = added > 0 || truncated || next.length !== prev.length;
      if (!changed) {
        for (let i = 0; i < page.length; i++) {
          const a = page[i];
          const b = prevById.get(a.requestId);
          if (
            !b ||
            a.statusCode !== b.statusCode ||
            a.latencyMs !== b.latencyMs ||
            a.firstTokenMs !== b.firstTokenMs ||
            a.inputTokens !== b.inputTokens ||
            a.outputTokens !== b.outputTokens ||
            a.cacheTokens !== b.cacheTokens
          ) {
            changed = true;
            break;
          }
        }
      }
      if (!changed) return;

      const el = listRef.current;
      const stickTop = !el || el.scrollTop <= STICK_TOP_PX;
      const prevScroll = el?.scrollTop ?? 0;
      if (stickTop) {
        pendingScrollTop.current = 0;
      } else if (added > 0) {
        // Keep the same rows under the user's eyes after prepend.
        pendingScrollTop.current = prevScroll + added * ROW_HEIGHT;
      } else {
        pendingScrollTop.current = null;
      }

      setLogs(next);
      if (truncated) setHasMore(true);
    } catch {
      /* silent — manual refresh still surfaces errors */
    } finally {
      pollInFlight.current = false;
    }
  }, []);

  useLayoutEffect(() => {
    const target = pendingScrollTop.current;
    if (target == null) return;
    pendingScrollTop.current = null;
    const box = listRef.current;
    if (!box) return;
    box.scrollTop = target;
    setScrollTop(target);
  }, [logs]);

  const refresh = useCallback(async () => {
    setHasMore(true);
    resetScroll();
    try {
      const list = await api.getAccounts();
      setAccounts(list);
    } catch {
      /* non-fatal for logs */
    }
    await loadPage(0, true);
  }, [loadPage, resetScroll]);

  async function loadManage() {
    try {
      const [s, c] = await Promise.all([api.getLogStats(), api.getConfig()]);
      setStats(s);
      setConfig(c);
      setRetentionDays(c.logRetentionDays ?? s.retentionDays ?? 30);
      setMaxRows(c.logMaxRows ?? s.maxRows ?? 50_000);
      if (!rangeFrom && s.oldestAt) setRangeFrom(toDateInputValue(s.oldestAt));
      if (!rangeTo && s.newestAt) setRangeTo(toDateInputValue(s.newestAt));
    } catch (e) {
      toast(String(e), "error");
    }
  }

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Live tail while this page is mounted; pause when tab hidden.
  useEffect(() => {
    const id = window.setInterval(() => {
      void pollLatest();
    }, POLL_MS);
    const onVis = () => {
      if (document.visibilityState === "visible") void pollLatest();
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [pollLatest]);

  useEffect(() => {
    if (manageOpen) void loadManage();
  }, [manageOpen]);

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
    if (nearBottom && hasMore && !loadingMore.current && logs.length < MAX_LOADED) {
      void loadPage(logs.length, false);
    }
  }

  async function afterMutation(deleted?: number) {
    if (deleted != null) {
      toast(t.logs.deleted.replace("{n}", String(deleted)), "success");
    }
    await loadManage();
    await refresh();
  }

  async function savePolicy() {
    if (!config) return;
    setBusy(true);
    try {
      const next = await api.updateConfig({
        ...config,
        logRetentionDays: Math.max(1, Number(retentionDays) || 30),
        logMaxRows: Math.max(0, Number(maxRows) || 0),
      });
      setConfig(next);
      toast(t.common.saved, "success");
      await loadManage();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  }

  async function runPruneNow() {
    setBusy(true);
    try {
      const s = await api.pruneLogsNow();
      setStats(s);
      toast(t.common.saved, "success");
      await refresh();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  }

  async function runClearAll() {
    if (!window.confirm(t.logs.confirmClearAll)) return;
    setBusy(true);
    try {
      await api.clearLogs();
      await afterMutation();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  }

  async function runClearOlder(days: number) {
    setBusy(true);
    try {
      const n = await api.clearLogsOlderThan(days);
      await afterMutation(n);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  }

  async function runClearRange() {
    if (!rangeFrom || !rangeTo) return;
    setBusy(true);
    try {
      const n = await api.clearLogsRange(rangeFrom, rangeTo);
      await afterMutation(n);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(false);
    }
  }

  const total = logs.length;
  const rawStart = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const start = total === 0 ? 0 : Math.min(rawStart, total - 1);
  const visibleCount = Math.ceil(Math.max(viewportH, 1) / ROW_HEIGHT) + OVERSCAN * 2;
  const end = Math.min(total, start + visibleCount);
  const visible = logs.slice(start, end);
  const padTop = Math.min(start * ROW_HEIGHT, Math.max(0, total * ROW_HEIGHT));
  const padBottom = Math.max(0, (total - end) * ROW_HEIGHT);
  const contentHeight = total * ROW_HEIGHT + (hasMore || loading ? 36 : 0);

  // Account/time | Source/Endpoint | Model | Latency (ttft/total) | Token | Cost
  const gridCols =
    "grid-cols-[minmax(196px,1.35fr)_minmax(120px,1.1fr)_minmax(88px,0.85fr)_minmax(72px,0.7fr)_minmax(200px,1.7fr)_60px]";

  return (
    <PageShell className="gap-3">
      <PageHeader>
        <h1 className="text-xl font-semibold tracking-tight">{t.logs.title}</h1>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => void refresh()}>
            {t.common.refresh}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setManageOpen(true)}
          >
            <Settings2 className="mr-1 h-3.5 w-3.5" />
            {t.logs.manage}
          </Button>
        </div>
      </PageHeader>

      <Card className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <CardContent className="flex min-h-0 flex-1 flex-col p-0">
          <div className="shrink-0 border-b border-neutral-200 px-4">
            <div className={`grid ${gridCols} gap-2 py-2 text-xs font-medium text-neutral-500`}>
              <div>{t.logs.accountTime}</div>
              <div>
                {t.logs.source}
                <span className="font-normal text-neutral-400"> / {t.logs.endpoint}</span>
              </div>
              <div>{t.logs.model}</div>
              <div title={`${t.logs.ttft} / ${t.logs.totalMs}`}>{t.logs.latency}</div>
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
              <EmptyState
                icon={Inbox}
                title={t.logs.empty}
                description={t.logs.emptyHint}
              />
            ) : (
              <div style={{ height: contentHeight, position: "relative" }}>
                <div style={{ height: padTop, flexShrink: 0 }} aria-hidden />
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
                      <div
                        className="min-w-0 leading-tight"
                        title={[log.clientSource || "—", log.endpoint].filter(Boolean).join("\n")}
                      >
                        <div
                          className="truncate text-[11px] font-medium text-neutral-800"
                          title={log.clientSource || undefined}
                        >
                          {log.clientSource || "—"}
                        </div>
                        <div
                          className="mt-0.5 truncate font-mono text-[10px] text-neutral-500"
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
                      <LatencyCell
                        log={log}
                        labels={{ ttft: t.logs.ttft, total: t.logs.totalMs }}
                      />
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

      <Dialog
        open={manageOpen}
        title={t.logs.manageTitle}
        onClose={() => setManageOpen(false)}
        className="max-w-lg"
        footer={
          <div className="flex justify-end gap-2">
            <Button variant="outline" size="sm" onClick={() => setManageOpen(false)}>
              {t.common.done}
            </Button>
          </div>
        }
      >
        <div className="space-y-5 text-sm">
          <section className="space-y-2">
            <div className="text-xs font-medium text-neutral-500">{t.logs.stats}</div>
            <div className="grid grid-cols-2 gap-2 rounded-lg border border-neutral-200 p-3 text-xs">
              <div>
                <span className="text-neutral-400">{t.logs.totalRows}</span>
                <div className="tabular-nums font-medium">
                  {stats ? formatNumber(stats.totalRows) : "—"}
                </div>
              </div>
              <div>
                <span className="text-neutral-400">{t.logs.dbSize}</span>
                <div className="tabular-nums font-medium">
                  {stats ? formatBytes(stats.dbBytes) : "—"}
                </div>
              </div>
              <div className="col-span-2 min-w-0">
                <span className="text-neutral-400">{t.logs.oldest}</span>
                <div className="truncate tabular-nums text-neutral-700">
                  {stats?.oldestAt
                    ? new Date(stats.oldestAt).toLocaleString(
                        locale === "zh-CN" ? "zh-CN" : "en-US"
                      )
                    : "—"}
                </div>
              </div>
              <div className="col-span-2 min-w-0">
                <span className="text-neutral-400">{t.logs.newest}</span>
                <div className="truncate tabular-nums text-neutral-700">
                  {stats?.newestAt
                    ? new Date(stats.newestAt).toLocaleString(
                        locale === "zh-CN" ? "zh-CN" : "en-US"
                      )
                    : "—"}
                </div>
              </div>
            </div>
          </section>

          <section className="space-y-2">
            <div className="text-xs font-medium text-neutral-500">{t.logs.savePolicy}</div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label className="text-xs">{t.logs.retention}</Label>
                <Input
                  type="number"
                  min={1}
                  className="mt-1"
                  value={retentionDays}
                  onChange={(e) => setRetentionDays(Math.max(1, Number(e.target.value) || 1))}
                />
              </div>
              <div>
                <Label className="text-xs">{t.logs.maxRows}</Label>
                <Input
                  type="number"
                  min={0}
                  className="mt-1"
                  value={maxRows}
                  onChange={(e) => setMaxRows(Math.max(0, Number(e.target.value) || 0))}
                />
                <p className="mt-0.5 text-[10px] text-neutral-400">{t.logs.maxRowsHint}</p>
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button size="sm" disabled={busy} onClick={() => void savePolicy()}>
                {t.common.save}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void runPruneNow()}
              >
                {t.logs.pruneNow}
              </Button>
            </div>
          </section>

          <section className="space-y-2">
            <div className="text-xs font-medium text-neutral-500">{t.logs.clearOlder}</div>
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void runClearOlder(1)}
              >
                {t.logs.days1}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void runClearOlder(7)}
              >
                {t.logs.days7}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void runClearOlder(30)}
              >
                {t.logs.days30}
              </Button>
            </div>
          </section>

          <section className="space-y-2">
            <div className="text-xs font-medium text-neutral-500">{t.logs.range}</div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <Label className="text-xs">{t.logs.rangeFrom}</Label>
                <Input
                  type="date"
                  className="mt-1"
                  value={rangeFrom}
                  onChange={(e) => setRangeFrom(e.target.value)}
                />
              </div>
              <div>
                <Label className="text-xs">{t.logs.rangeTo}</Label>
                <Input
                  type="date"
                  className="mt-1"
                  value={rangeTo}
                  onChange={(e) => setRangeTo(e.target.value)}
                />
              </div>
            </div>
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !rangeFrom || !rangeTo}
              onClick={() => void runClearRange()}
            >
              {t.logs.clearRange}
            </Button>
          </section>

          <section>
            <Button
              size="sm"
              variant="destructive"
              disabled={busy}
              onClick={() => void runClearAll()}
            >
              {t.logs.clearAll}
            </Button>
          </section>
        </div>
      </Dialog>
    </PageShell>
  );
}
