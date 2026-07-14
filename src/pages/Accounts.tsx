import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  CheckSquare,
  Image as ImageIcon,
  KeyRound,
  RefreshCw,
  SearchX,
  Square,
  TimerOff,
  Trash2,
  Upload,
  Users,
  Video,
} from "lucide-react";
import {
  api,
  type Account,
  type AccountQuota,
  type ImportAccountsOptions,
} from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ConfirmDialog, Dialog } from "@/components/ui/dialog";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { PageBody, PageHeader, PageShell } from "@/components/page-shell";
import { Select } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useI18n } from "@/i18n/context";
import { useToast } from "@/components/ui/toast";
import { cn } from "@/lib/utils";

function formatShortDateTime(iso: string | null | undefined, locale: string): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  try {
    return new Intl.DateTimeFormat(locale, {
      month: "numeric",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(d);
  } catch {
    return d.toLocaleString();
  }
}

function formatFullDateTime(iso: string | null | undefined, locale: string): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  try {
    return new Intl.DateTimeFormat(locale, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    }).format(d);
  } catch {
    return d.toLocaleString();
  }
}

function formatPercent(n: number | null | undefined): string {
  if (n == null || Number.isNaN(n)) return "—";
  const rounded = Math.round(n * 10) / 10;
  return Number.isInteger(rounded) ? `${rounded}%` : `${rounded.toFixed(1)}%`;
}

function barTone(usedPercent: number): string {
  if (usedPercent >= 90) return "bg-red-500";
  if (usedPercent >= 70) return "bg-amber-500";
  return "bg-emerald-500";
}

function remainingTone(
  usedPercent: number | null | undefined
): "success" | "warning" | "danger" | "outline" {
  if (usedPercent == null) return "outline";
  if (usedPercent >= 90) return "danger";
  if (usedPercent >= 70) return "warning";
  return "success";
}

function supportsImage(account: Account): boolean {
  return account.supportsImage !== false;
}

function supportsVideo(account: Account): boolean {
  return account.supportsVideo !== false;
}

/** Compact tag: short text, full label via native title (hover). */
function Tag({
  children,
  title,
  variant = "outline",
  className,
}: {
  children: React.ReactNode;
  title: string;
  variant?: "default" | "success" | "warning" | "danger" | "outline";
  className?: string;
}) {
  return (
    <Badge
      variant={variant}
      title={title}
      className={cn("max-w-full truncate px-1.5 py-0 text-[11px] leading-5", className)}
    >
      {children}
    </Badge>
  );
}

function IconAction({
  title,
  onClick,
  disabled,
  variant = "outline",
  children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  variant?: "outline" | "destructive" | "ghost";
  children: React.ReactNode;
}) {
  return (
    <Button
      type="button"
      size="icon"
      variant={variant}
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className="h-8 w-8 shrink-0"
    >
      {children}
    </Button>
  );
}

function AccountCard({
  account,
  locale,
  busy,
  quotaBusy,
  selected,
  labels,
  onToggleSelect,
  onSave,
  onRemove,
  onClearCooldown,
  onRelogin,
  onRefreshQuota,
}: {
  account: Account;
  locale: string;
  busy: boolean;
  quotaBusy: boolean;
  selected: boolean;
  labels: {
    enabled: string;
    weight: string;
    loggedIn: string;
    notLoggedIn: string;
    health: (h: Account["health"]) => string;
    remaining: string;
    used: string;
    resetsAt: string;
    periodStart: string;
    fetchedAt: string;
    unknown: string;
    needLogin: string;
    error: string;
    unusedQuota: string;
    unusedQuotaShort: string;
    rateLimit: string;
    refreshQuota: string;
    refreshingQuota: string;
    relogin: string;
    loggingIn: string;
    clearCooldown: string;
    remove: string;
    imageOn: string;
    imageOff: string;
    videoOn: string;
    videoOff: string;
  };
  onToggleSelect: (id: string) => void;
  onSave: (account: Account) => void;
  onRemove: (id: string) => void;
  onClearCooldown: (id: string) => void;
  onRelogin: (id: string) => void;
  onRefreshQuota: (id: string) => void;
}) {
  const signedIn = Boolean(account.accessToken || account.refreshToken);
  const quota: AccountQuota | null | undefined = account.quota;
  // Soft-heal legacy bad stamps: parse failure stored used=0 remaining=0 + lastError.
  const softUnused =
    !!quota?.lastError &&
    (quota.usedPercent ?? 0) === 0 &&
    (quota.remainingPercent ?? 0) === 0;
  const used = softUnused ? 0 : (quota?.usedPercent ?? null);
  const remaining = softUnused
    ? 100
    : (quota?.remainingPercent ??
      (used == null ? null : Math.max(0, 100 - used)));
  const usedClamped = Math.max(0, Math.min(100, used ?? 0));
  const hasQuota = signedIn && quota != null && used != null;
  const showHardQuotaError =
    !!quota?.lastError &&
    !softUnused &&
    !/could not parse quota percent/i.test(quota.lastError);
  // Fresh SuperGrok week with no product breakdown (common for CPA / never SuperGrok-billed).
  const superGrokEmpty =
    hasQuota &&
    used === 0 &&
    remaining === 100 &&
    (quota?.products?.length ?? 0) === 0;
  const name = account.email || account.name;
  const imgOk = supportsImage(account);
  const vidOk = supportsVideo(account);
  const hasRateLimit =
    account.rateLimitRemaining != null || account.rateLimitLimit != null;

  const healthVariant =
    account.health === "healthy"
      ? "success"
      : account.health === "degraded"
        ? "warning"
        : "danger";

  const productTags = (quota?.products ?? [])
    .filter((p) => p.usedPercent > 0 || p.productId === 1 || p.productId === 2)
    .slice(0, 3);

  return (
    <Card className={cn("overflow-hidden", selected && "ring-2 ring-neutral-900/15")}>
      <CardContent className="p-0">
        <div className="grid grid-cols-1 gap-2 px-3 py-2.5 sm:grid-cols-[auto_minmax(0,1.2fr)_minmax(0,1fr)_auto] sm:items-center sm:gap-3">
          <button
            type="button"
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-neutral-200 text-neutral-600 hover:bg-neutral-50"
            title={selected ? "Unselect" : "Select"}
            aria-pressed={selected}
            onClick={() => onToggleSelect(account.id)}
          >
            {selected ? (
              <CheckSquare className="h-4 w-4" />
            ) : (
              <Square className="h-4 w-4" />
            )}
          </button>

          {/* Identity */}
          <div className="flex min-w-0 items-center gap-2">
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium leading-5" title={name}>
                {name}
              </div>
              <div className="mt-1 flex flex-wrap items-center gap-1">
                <Tag title={labels.health(account.health)} variant={healthVariant}>
                  {labels.health(account.health)}
                </Tag>
                <span
                  className={cn(
                    "inline-flex h-5 w-5 items-center justify-center rounded-full border",
                    signedIn
                      ? "border-emerald-200 bg-emerald-50"
                      : "border-amber-200 bg-amber-50"
                  )}
                  title={signedIn ? labels.loggedIn : labels.notLoggedIn}
                  aria-label={signedIn ? labels.loggedIn : labels.notLoggedIn}
                >
                  <span
                    className={cn(
                      "h-1.5 w-1.5 rounded-full",
                      signedIn ? "bg-emerald-500" : "bg-amber-400"
                    )}
                  />
                </span>
                {account.authKind === "sso" ? (
                  <Tag
                    title="Legacy SSO — needs convert to OAuth (re-import or convert)"
                    variant="warning"
                  >
                    SSO→OAuth
                  </Tag>
                ) : null}
                <Tag
                  title={imgOk ? labels.imageOn : labels.imageOff}
                  variant={imgOk ? "success" : "outline"}
                  className="cursor-pointer"
                >
                  <button
                    type="button"
                    className="inline-flex items-center gap-0.5"
                    onClick={() =>
                      onSave({ ...account, supportsImage: !imgOk })
                    }
                  >
                    <ImageIcon className="h-3 w-3" />
                    {imgOk ? "图" : "图×"}
                  </button>
                </Tag>
                <Tag
                  title={vidOk ? labels.videoOn : labels.videoOff}
                  variant={vidOk ? "success" : "outline"}
                >
                  <button
                    type="button"
                    className="inline-flex items-center gap-0.5"
                    onClick={() =>
                      onSave({ ...account, supportsVideo: !vidOk })
                    }
                  >
                    <Video className="h-3 w-3" />
                    {vidOk ? "视" : "视×"}
                  </button>
                </Tag>
                {showHardQuotaError ? (
                  <Tag title={`${labels.error}: ${quota?.lastError}`} variant="danger">
                    Err
                  </Tag>
                ) : softUnused || superGrokEmpty ? (
                  <Tag title={labels.unusedQuota} variant="outline">
                    {labels.unusedQuotaShort}
                  </Tag>
                ) : null}
              </div>
            </div>
          </div>

          {/* Quota */}
          <div className="min-w-0 space-y-1.5">
            <div className="h-1.5 overflow-hidden rounded-full bg-neutral-100">
              <div
                className={cn(
                  "h-full rounded-full transition-[width]",
                  hasQuota ? barTone(usedClamped) : "bg-neutral-200"
                )}
                style={{ width: hasQuota ? `${usedClamped}%` : "0%" }}
              />
            </div>
            <div className="flex min-h-5 flex-wrap items-center gap-1">
              {/* API rate limit — updated on chat traffic and on quota refresh probe */}
              {hasRateLimit ? (
                <Tag
                  title={`${labels.rateLimit}: ${
                    account.rateLimitRemaining != null && account.rateLimitLimit != null
                      ? `${account.rateLimitRemaining}/${account.rateLimitLimit}`
                      : String(account.rateLimitRemaining ?? account.rateLimitLimit ?? "—")
                  }${
                    account.rateLimitResetAt
                      ? ` · ${formatFullDateTime(account.rateLimitResetAt, locale)}`
                      : ""
                  }`}
                  variant="success"
                >
                  API{" "}
                  {account.rateLimitRemaining != null && account.rateLimitLimit != null
                    ? `${account.rateLimitRemaining}/${account.rateLimitLimit}`
                    : String(account.rateLimitRemaining ?? account.rateLimitLimit ?? "—")}
                </Tag>
              ) : null}
              {hasQuota ? (
                <>
                  <Tag
                    title={`SuperGrok ${labels.remaining} ${formatPercent(remaining)} · ${labels.used} ${formatPercent(used)}`}
                    variant={remainingTone(used)}
                  >
                    SG {formatPercent(remaining)}
                  </Tag>
                  <Tag
                    title={`${labels.resetsAt}: ${formatFullDateTime(quota?.resetsAt, locale)}${
                      quota?.periodStartAt
                        ? ` · ${labels.periodStart}: ${formatFullDateTime(quota.periodStartAt, locale)}`
                        : ""
                    }${
                      quota?.fetchedAt
                        ? ` · ${labels.fetchedAt}: ${formatFullDateTime(quota.fetchedAt, locale)}`
                        : ""
                    }`}
                  >
                    ↻ {formatShortDateTime(quota?.resetsAt, locale)}
                  </Tag>
                  {productTags.map((p) => (
                    <Tag
                      key={`${p.productId}-${p.label}`}
                      title={`${p.label}: ${labels.used} ${formatPercent(p.usedPercent)}`}
                    >
                      {p.label} {formatPercent(p.usedPercent)}
                    </Tag>
                  ))}
                </>
              ) : !hasRateLimit ? (
                <Tag title={signedIn ? labels.unknown : labels.needLogin}>—</Tag>
              ) : null}
            </div>
          </div>

          {/* Controls */}
          <div className="flex flex-wrap items-center justify-end gap-1.5 sm:flex-nowrap">
            <div className="flex h-8 items-center gap-1.5 rounded-md border border-neutral-200 px-2">
              <span className="select-none text-xs text-neutral-500">{labels.enabled}</span>
              <Switch
                checked={account.enabled}
                onCheckedChange={(v) => onSave({ ...account, enabled: v })}
                aria-label={labels.enabled}
              />
            </div>
            <div className="flex h-8 items-center gap-1 rounded-md border border-neutral-200 px-1.5">
              <span className="select-none text-xs text-neutral-500">{labels.weight}</span>
              <Input
                className="h-7 w-12 border-0 px-1 text-center text-xs shadow-none focus:ring-0"
                type="number"
                min={1}
                value={account.weight}
                aria-label={labels.weight}
                onChange={(e) =>
                  onSave({
                    ...account,
                    weight: Math.max(1, Number(e.target.value) || 1),
                  })
                }
              />
            </div>

            <IconAction
              title={quotaBusy ? labels.refreshingQuota : labels.refreshQuota}
              disabled={!signedIn || quotaBusy || busy}
              onClick={() => onRefreshQuota(account.id)}
            >
              <RefreshCw className={cn("h-3.5 w-3.5", quotaBusy && "animate-spin")} />
            </IconAction>

            {account.health === "cooldown" ? (
              <IconAction
                title={labels.clearCooldown}
                disabled={busy}
                onClick={() => onClearCooldown(account.id)}
              >
                <TimerOff className="h-3.5 w-3.5" />
              </IconAction>
            ) : (
              <span className="hidden h-8 w-8 sm:inline-block" aria-hidden />
            )}

            <IconAction
              title={busy ? labels.loggingIn : labels.relogin}
              disabled={busy}
              onClick={() => onRelogin(account.id)}
            >
              <KeyRound className="h-3.5 w-3.5" />
            </IconAction>

            <IconAction
              title={labels.remove}
              disabled={busy}
              variant="destructive"
              onClick={() => onRemove(account.id)}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </IconAction>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export function AccountsPage() {
  const { t, locale } = useI18n();
  const { toast } = useToast();
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState("");
  const [authUrl, setAuthUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [quotaBusyId, setQuotaBusyId] = useState<string | null>(null);
  const [quotaBusyAll, setQuotaBusyAll] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [filterMedia, setFilterMedia] = useState<"all" | "image" | "video" | "text-only">("all");
  const [importOpen, setImportOpen] = useState(false);
  const [importText, setImportText] = useState("");
  const [importBusy, setImportBusy] = useState(false);
  const [importOpts, setImportOpts] = useState<ImportAccountsOptions>({
    weight: 1,
    supportsImage: true,
    supportsVideo: true,
    skipDuplicates: true,
    validateRefresh: true,
  });
  const [bulkWeight, setBulkWeight] = useState("1");
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const waitingAccountId = useRef<string | null>(null);
  const autoQuotaRef = useRef(false);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const stopPoll = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
    waitingAccountId.current = null;
  }, []);

  const refresh = useCallback(async () => {
    try {
      const list = await api.getAccounts();
      setAccounts(list);
      const waitId = waitingAccountId.current;
      if (waitId) {
        const acc = list.find((a) => a.id === waitId);
        if (acc?.accessToken) {
          stopPoll();
          setAuthUrl("");
          toast(
            acc.email
              ? t.accounts.loginSuccessWithEmail.replace("{email}", acc.email)
              : t.accounts.loginSuccess,
            "success"
          );
          try {
            setAccounts(await api.refreshAccountQuota(acc.id));
          } catch {
            /* non-fatal */
          }
        }
      }
      return list;
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
      return null;
    }
  }, [stopPoll, t.accounts.loginSuccess, t.accounts.loginSuccessWithEmail, toast]);

  useEffect(() => {
    void (async () => {
      const list = await refresh();
      if (!list || autoQuotaRef.current) return;
      autoQuotaRef.current = true;
      const need = list.some((a) => a.accessToken && !a.quota);
      if (!need) return;
      setQuotaBusyAll(true);
      try {
        setAccounts(await api.refreshAllAccountQuotas());
      } catch (e) {
        toast(String(e), "warning");
      } finally {
        setQuotaBusyAll(false);
      }
    })();
    return () => stopPoll();
  }, [refresh, stopPoll, toast]);

  useEffect(() => {
    const onFocus = () => {
      void refresh();
    };
    const onVis = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [refresh]);

  // Drop selection for removed accounts.
  useEffect(() => {
    setSelected((prev) => {
      const ids = new Set(accounts.map((a) => a.id));
      const next = new Set<string>();
      for (const id of prev) {
        if (ids.has(id)) next.add(id);
      }
      return next.size === prev.size ? prev : next;
    });
  }, [accounts]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return accounts.filter((a) => {
      if (filterMedia === "image" && !supportsImage(a)) return false;
      if (filterMedia === "video" && !supportsVideo(a)) return false;
      if (filterMedia === "text-only" && (supportsImage(a) || supportsVideo(a))) return false;
      if (!q) return true;
      const hay = `${a.email ?? ""} ${a.name} ${a.notes ?? ""}`.toLowerCase();
      return hay.includes(q);
    });
  }, [accounts, filterMedia, query]);

  const allFilteredSelected =
    filtered.length > 0 && filtered.every((a) => selected.has(a.id));

  function startWaitingFor(accountId: string) {
    stopPoll();
    waitingAccountId.current = accountId;
    let ticks = 0;
    pollRef.current = setInterval(() => {
      ticks += 1;
      void refresh();
      if (ticks >= 90) {
        stopPoll();
        toast(t.accounts.loginStillPending, "warning");
      }
    }, 2000);
  }

  async function login(opts?: { accountId?: string }) {
    if (busy) return;
    setBusy(true);
    setError("");
    setAuthUrl("");
    toast(t.accounts.loggingIn, "info");
    try {
      const start = await api.startOAuthLogin({
        accountId: opts?.accountId,
      });
      setAuthUrl(start.authorizeUrl);
      let opened = start.browserOpened;
      if (!opened) {
        try {
          await openUrl(start.authorizeUrl);
          opened = true;
        } catch {
          // ignore
        }
      }
      toast(
        opened ? t.accounts.browserOpened : t.accounts.browserOpenFailed,
        opened ? "success" : "warning"
      );
      startWaitingFor(start.accountId);
      await refresh();
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
      stopPoll();
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function openAuthLink() {
    if (!authUrl) return;
    try {
      await openUrl(authUrl);
    } catch {
      try {
        await navigator.clipboard.writeText(authUrl);
        toast(t.accounts.urlCopied, "info");
      } catch {
        setError(authUrl);
        toast(authUrl, "error");
      }
    }
  }

  async function save(account: Account) {
    try {
      setAccounts(await api.upsertAccount(account));
      setError("");
      toast(t.common.saved, "success");
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    }
  }

  async function remove(id: string) {
    try {
      setAccounts(await api.deleteAccount(id));
      setError("");
      toast(t.common.remove, "success");
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    }
  }

  async function clearCooldown(id: string) {
    try {
      setAccounts(await api.clearAccountCooldown(id));
      setError("");
      toast(t.common.saved, "success");
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    }
  }

  async function refreshQuota(id: string) {
    setQuotaBusyId(id);
    setError("");
    try {
      const list = await api.refreshAccountQuota(id);
      setAccounts(list);
      const acc = list.find((a) => a.id === id);
      const sg = acc?.quota;
      const rl =
        acc?.rateLimitRemaining != null && acc?.rateLimitLimit != null
          ? `API ${acc.rateLimitRemaining}/${acc.rateLimitLimit}`
          : null;
      const superPart =
        sg != null
          ? `SuperGrok ${formatPercent(sg.remainingPercent ?? Math.max(0, 100 - (sg.usedPercent ?? 0)))} ${t.accounts.remaining}`
          : null;
      toast(
        [rl, superPart].filter(Boolean).join(" · ") || t.common.saved,
        "success"
      );
    } catch (e) {
      try {
        setAccounts(await api.getAccounts());
      } catch {
        /* ignore */
      }
      setError(String(e));
      toast(String(e), "error");
    } finally {
      setQuotaBusyId(null);
    }
  }

  async function refreshAllQuotas() {
    setQuotaBusyAll(true);
    setError("");
    try {
      setAccounts(await api.refreshAllAccountQuotas());
      toast(t.common.saved, "success");
    } catch (e) {
      try {
        setAccounts(await api.getAccounts());
      } catch {
        /* ignore */
      }
      setError(String(e));
      toast(String(e), "error");
    } finally {
      setQuotaBusyAll(false);
    }
  }

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  function toggleSelectAllFiltered() {
    if (allFilteredSelected) {
      setSelected((prev) => {
        const next = new Set(prev);
        for (const a of filtered) next.delete(a.id);
        return next;
      });
    } else {
      setSelected((prev) => {
        const next = new Set(prev);
        for (const a of filtered) next.add(a.id);
        return next;
      });
    }
  }

  async function runBatchPatch(
    patch: Parameters<typeof api.batchPatchAccounts>[1],
    emptyMsg?: string
  ) {
    const ids = [...selected];
    if (ids.length === 0) {
      toast(emptyMsg ?? t.accounts.bulkNeedSelection, "warning");
      return;
    }
    try {
      setAccounts(await api.batchPatchAccounts(ids, patch));
      toast(t.accounts.bulkUpdated.replace("{count}", String(ids.length)), "success");
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    }
  }

  function requestBatchDelete() {
    if (selected.size === 0) {
      toast(t.accounts.bulkNeedSelection, "warning");
      return;
    }
    // Do not use window.confirm — WKWebView/Tauri often returns undefined/false
    // without showing a dialog, which silently aborts the delete.
    setDeleteConfirmOpen(true);
  }

  async function runBatchDelete() {
    const ids = [...selected].filter((id) => typeof id === "string" && id.trim().length > 0);
    if (ids.length === 0) {
      toast(t.accounts.bulkNeedSelection, "warning");
      setDeleteConfirmOpen(false);
      return;
    }
    setDeleteBusy(true);
    setError("");
    try {
      const next = await api.batchDeleteAccounts(ids);
      setAccounts(next);
      setSelected(new Set());
      setDeleteConfirmOpen(false);
      toast(t.accounts.bulkDeleted.replace("{count}", String(ids.length)), "success");
    } catch (e) {
      // Refresh list in case partial / stale selection.
      try {
        setAccounts(await api.getAccounts());
      } catch {
        /* ignore */
      }
      setError(String(e));
      toast(String(e), "error");
    } finally {
      setDeleteBusy(false);
    }
  }

  async function runImport() {
    const payload = importText.trim();
    if (!payload) {
      toast(t.accounts.importEmpty, "warning");
      return;
    }
    setImportBusy(true);
    setError("");
    try {
      const result = await api.importAccounts(payload, importOpts);
      setAccounts(result.accounts);
      const summary = t.accounts.importResult
        .replace("{added}", String(result.added))
        .replace("{skipped}", String(result.skipped))
        .replace("{failed}", String(result.failed));
      if (result.failed > 0 && result.added === 0) {
        toast(summary, "error");
        setError(result.errors.map((e) => `#${e.index}: ${e.detail}`).join("\n"));
      } else if (result.failed > 0) {
        toast(summary, "warning");
        setError(result.errors.map((e) => `#${e.index}: ${e.detail}`).join("\n"));
      } else {
        toast(summary, "success");
        setImportOpen(false);
        setImportText("");
      }
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    } finally {
      setImportBusy(false);
    }
  }

  async function onPickImportFiles(files: FileList | null) {
    if (!files?.length) return;
    const parts: string[] = [];
    for (const file of Array.from(files)) {
      try {
        parts.push(await file.text());
      } catch (e) {
        toast(`${file.name}: ${String(e)}`, "error");
      }
    }
    // Prefer JSON array merge when all are JSON objects.
    const objects: unknown[] = [];
    let allJson = true;
    for (const p of parts) {
      try {
        const v = JSON.parse(p);
        if (Array.isArray(v)) objects.push(...v);
        else objects.push(v);
      } catch {
        allJson = false;
        break;
      }
    }
    setImportText(allJson ? JSON.stringify(objects, null, 2) : parts.join("\n"));
  }

  const dateLocale = locale === "en" ? "en-US" : "zh-CN";
  const hasSignedIn = accounts.some((a) => a.accessToken || a.refreshToken);
  const hasLegacySso = accounts.some(
    (a) => a.authKind === "sso" || (a.ssoToken && !a.refreshToken)
  );

  async function runConvertSso() {
    setImportBusy(true);
    setError("");
    try {
      const result = await api.convertSsoAccounts();
      setAccounts(result.accounts);
      const summary = t.accounts.convertSsoResult
        .replace("{added}", String(result.added))
        .replace("{skipped}", String(result.skipped))
        .replace("{failed}", String(result.failed));
      if (result.added === 0 && result.failed === 0) {
        toast(t.accounts.convertSsoNone, "warning");
      } else if (result.failed > 0 && result.added === 0) {
        toast(summary, "error");
        setError(result.errors.map((e) => `#${e.index}: ${e.detail}`).join("\n"));
      } else if (result.failed > 0) {
        toast(summary, "warning");
        setError(result.errors.map((e) => `#${e.index}: ${e.detail}`).join("\n"));
      } else {
        toast(summary, "success");
      }
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    } finally {
      setImportBusy(false);
    }
  }

  const cardLabels = {
    enabled: t.common.enabled,
    weight: t.common.weight,
    loggedIn: t.accounts.loggedIn,
    notLoggedIn: t.accounts.notLoggedIn,
    health: (h: Account["health"]) => t.accounts.health[h] ?? h,
    remaining: t.accounts.remaining,
    used: t.accounts.used,
    resetsAt: t.accounts.resetsAt,
    periodStart: t.accounts.periodStart,
    fetchedAt: t.accounts.quotaFetchedAt,
    unknown: t.accounts.quotaUnknown,
    needLogin: t.accounts.quotaNeedLogin,
    error: t.accounts.quotaError,
    unusedQuota: t.accounts.quotaUnused,
    unusedQuotaShort: t.accounts.quotaUnusedShort,
    rateLimit: t.accounts.rateLimitHint,
    refreshQuota: t.accounts.refreshQuota,
    refreshingQuota: t.accounts.refreshingQuota,
    relogin: t.accounts.relogin,
    loggingIn: t.accounts.loggingIn,
    clearCooldown: t.accounts.clearCooldown,
    remove: t.common.remove,
    imageOn: t.accounts.imageOn,
    imageOff: t.accounts.imageOff,
    videoOn: t.accounts.videoOn,
    videoOff: t.accounts.videoOff,
  };

  return (
    <PageShell>
      <PageHeader>
        <div className="min-w-0 space-y-1">
          <h1 className="text-xl font-semibold tracking-tight">{t.accounts.title}</h1>
          {t.accounts.subtitle ? (
            <p className="text-sm text-neutral-500">{t.accounts.subtitle}</p>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {hasSignedIn ? (
            <Button
              variant="outline"
              size="sm"
              disabled={quotaBusyAll || busy}
              onClick={() => void refreshAllQuotas()}
              title={t.accounts.refreshAllQuotas}
            >
              <RefreshCw className={cn("h-3.5 w-3.5", quotaBusyAll && "animate-spin")} />
              {quotaBusyAll ? t.accounts.refreshingQuota : t.accounts.refreshAllQuotas}
            </Button>
          ) : null}
          {hasLegacySso ? (
            <Button
              variant="outline"
              size="sm"
              disabled={busy || importBusy}
              onClick={() => void runConvertSso()}
              title={t.accounts.convertSsoTitle}
            >
              <KeyRound className={cn("h-3.5 w-3.5", importBusy && "animate-pulse")} />
              {t.accounts.convertSso}
            </Button>
          ) : null}
          <Button
            variant="outline"
            size="sm"
            disabled={busy || importBusy}
            onClick={() => setImportOpen(true)}
          >
            <Upload className="h-3.5 w-3.5" />
            {t.accounts.importAccounts}
          </Button>
          <Button size="sm" disabled={busy} onClick={() => login()}>
            {busy ? t.accounts.loggingIn : t.accounts.addAccount}
          </Button>
        </div>
      </PageHeader>

      {/* Search / filter */}
      <div className="flex shrink-0 flex-wrap items-center gap-2">
        <Input
          className="h-8 w-full max-w-xs text-sm"
          placeholder={t.accounts.searchPlaceholder}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <Select
          size="sm"
          className="w-[8.5rem] shrink-0"
          value={filterMedia}
          onChange={(e) =>
            setFilterMedia(e.target.value as typeof filterMedia)
          }
        >
          <option value="all">{t.accounts.filterAll}</option>
          <option value="image">{t.accounts.filterImage}</option>
          <option value="video">{t.accounts.filterVideo}</option>
          <option value="text-only">{t.accounts.filterTextOnly}</option>
        </Select>
        <span className="text-xs text-neutral-500">
          {t.accounts.shownCount
            .replace("{shown}", String(filtered.length))
            .replace("{total}", String(accounts.length))}
        </span>
        {filtered.length > 0 ? (
          <Button type="button" size="sm" variant="ghost" onClick={toggleSelectAllFiltered}>
            {allFilteredSelected ? t.accounts.unselectPage : t.accounts.selectPage}
          </Button>
        ) : null}
      </div>

      {/* Bulk actions */}
      {selected.size > 0 ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2 rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2">
          <span className="text-xs font-medium text-neutral-700">
            {t.accounts.bulkSelected.replace("{count}", String(selected.size))}
          </span>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runBatchPatch({ enabled: true })}
          >
            {t.accounts.bulkEnable}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runBatchPatch({ enabled: false })}
          >
            {t.accounts.bulkDisable}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runBatchPatch({ supportsImage: true })}
          >
            {t.accounts.bulkImageOn}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runBatchPatch({ supportsImage: false })}
          >
            {t.accounts.bulkImageOff}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runBatchPatch({ supportsVideo: true })}
          >
            {t.accounts.bulkVideoOn}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runBatchPatch({ supportsVideo: false })}
          >
            {t.accounts.bulkVideoOff}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runBatchPatch({ clearCooldown: true })}
          >
            {t.accounts.bulkClearCooldown}
          </Button>
          <div className="flex items-center gap-1">
            <Input
              className="h-8 w-14 text-center text-xs"
              type="number"
              min={1}
              value={bulkWeight}
              onChange={(e) => setBulkWeight(e.target.value)}
              title={t.common.weight}
            />
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                void runBatchPatch({
                  weight: Math.max(1, Number(bulkWeight) || 1),
                })
              }
            >
              {t.accounts.bulkSetWeight}
            </Button>
          </div>
          <Button
            size="sm"
            variant="destructive"
            disabled={deleteBusy}
            onClick={() => requestBatchDelete()}
          >
            {t.accounts.bulkDelete}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setSelected(new Set())}>
            {t.accounts.bulkClearSelection}
          </Button>
        </div>
      ) : null}

      {error ? (
        <div className="shrink-0 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 whitespace-pre-wrap">
          {error}
        </div>
      ) : null}
      {authUrl ? (
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <Button size="sm" onClick={openAuthLink}>
            {t.accounts.openAuthLink}
          </Button>
          <Button size="sm" variant="outline" onClick={() => refresh()}>
            {t.accounts.refreshStatus}
          </Button>
        </div>
      ) : null}

      <PageBody className="flex flex-col">
        {accounts.length === 0 ? (
          <EmptyState
            icon={Users}
            title={t.accounts.empty}
            description={t.accounts.emptyHint}
            action={
              <div className="flex flex-wrap justify-center gap-2">
                <Button size="sm" disabled={busy} onClick={() => login()}>
                  {busy ? t.accounts.loggingIn : t.accounts.addAccount}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy || importBusy}
                  onClick={() => setImportOpen(true)}
                >
                  <Upload className="h-3.5 w-3.5" />
                  {t.accounts.importAccounts}
                </Button>
              </div>
            }
          />
        ) : filtered.length === 0 ? (
          <EmptyState
            icon={SearchX}
            title={t.accounts.noMatch}
            description={t.accounts.noMatchHint}
          />
        ) : (
          <div className="space-y-2 pb-1">
            {filtered.map((account) => (
              <AccountCard
                key={account.id}
                account={account}
                locale={dateLocale}
                busy={busy}
                quotaBusy={quotaBusyAll || quotaBusyId === account.id}
                selected={selected.has(account.id)}
                labels={cardLabels}
                onToggleSelect={toggleSelect}
                onSave={(a) => void save(a)}
                onRemove={(id) => void remove(id)}
                onClearCooldown={(id) => void clearCooldown(id)}
                onRelogin={(id) => void login({ accountId: id })}
                onRefreshQuota={(id) => void refreshQuota(id)}
              />
            ))}
          </div>
        )}
      </PageBody>

      <ConfirmDialog
        open={deleteConfirmOpen}
        title={t.accounts.bulkDelete}
        description={t.accounts.bulkDeleteConfirm.replace(
          "{count}",
          String(selected.size)
        )}
        cancelLabel={t.common.cancel}
        confirmLabel={t.common.remove}
        busy={deleteBusy}
        onCancel={() => {
          if (!deleteBusy) setDeleteConfirmOpen(false);
        }}
        onConfirm={() => void runBatchDelete()}
      />

      {/* Import dialog */}
      <Dialog
        open={importOpen}
        title={t.accounts.importTitle}
        description={t.accounts.importHint || undefined}
        className="max-w-xl"
        onClose={importBusy ? undefined : () => setImportOpen(false)}
      >
        <div className="space-y-3">
          <Textarea
            className="min-h-[180px] font-mono text-xs"
            placeholder={t.accounts.importPlaceholder}
            value={importText}
            onChange={(e) => setImportText(e.target.value)}
            disabled={importBusy}
          />
          <div className="flex flex-wrap items-center gap-2">
            <input
              ref={fileInputRef}
              type="file"
              className="hidden"
              accept="application/json,.json,.txt,.ndjson"
              multiple
              onChange={(e) => {
                void onPickImportFiles(e.target.files);
                e.target.value = "";
              }}
            />
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={importBusy}
              onClick={() => fileInputRef.current?.click()}
            >
              {t.accounts.importPickFiles}
            </Button>
          </div>
          <div className="grid grid-cols-2 gap-2 text-xs sm:grid-cols-3">
            <label className="flex items-center gap-2 rounded-md border border-neutral-200 px-2 py-1.5">
              <span className="text-neutral-500">{t.common.weight}</span>
              <Input
                className="h-7 w-14 border-0 px-1 text-center shadow-none"
                type="number"
                min={1}
                value={importOpts.weight ?? 1}
                onChange={(e) =>
                  setImportOpts((o) => ({
                    ...o,
                    weight: Math.max(1, Number(e.target.value) || 1),
                  }))
                }
              />
            </label>
            <label className="flex items-center justify-between gap-2 rounded-md border border-neutral-200 px-2 py-1.5">
              <span>{t.accounts.importSupportsImage}</span>
              <Switch
                checked={importOpts.supportsImage !== false}
                onCheckedChange={(v) =>
                  setImportOpts((o) => ({ ...o, supportsImage: v }))
                }
              />
            </label>
            <label className="flex items-center justify-between gap-2 rounded-md border border-neutral-200 px-2 py-1.5">
              <span>{t.accounts.importSupportsVideo}</span>
              <Switch
                checked={importOpts.supportsVideo !== false}
                onCheckedChange={(v) =>
                  setImportOpts((o) => ({ ...o, supportsVideo: v }))
                }
              />
            </label>
            <label className="flex items-center justify-between gap-2 rounded-md border border-neutral-200 px-2 py-1.5">
              <span>{t.accounts.importSkipDup}</span>
              <Switch
                checked={importOpts.skipDuplicates !== false}
                onCheckedChange={(v) =>
                  setImportOpts((o) => ({ ...o, skipDuplicates: v }))
                }
              />
            </label>
            <label className="col-span-2 flex items-center justify-between gap-2 rounded-md border border-neutral-200 px-2 py-1.5 sm:col-span-2">
              <span>{t.accounts.importValidateRt}</span>
              <Switch
                checked={importOpts.validateRefresh !== false}
                onCheckedChange={(v) =>
                  setImportOpts((o) => ({ ...o, validateRefresh: v }))
                }
              />
            </label>
          </div>
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={importBusy}
              onClick={() => setImportOpen(false)}
            >
              {t.common.cancel}
            </Button>
            <Button
              type="button"
              disabled={importBusy || !importText.trim()}
              onClick={() => void runImport()}
            >
              {importBusy ? t.accounts.importing : t.accounts.importSubmit}
            </Button>
          </div>
        </div>
      </Dialog>
    </PageShell>
  );
}
