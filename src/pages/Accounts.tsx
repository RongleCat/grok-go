import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  KeyRound,
  RefreshCw,
  TimerOff,
  Trash2,
} from "lucide-react";
import { api, type Account, type AccountQuota } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
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
  labels,
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
    rateLimit: string;
    refreshQuota: string;
    refreshingQuota: string;
    relogin: string;
    loggingIn: string;
    clearCooldown: string;
    remove: string;
  };
  onSave: (account: Account) => void;
  onRemove: (id: string) => void;
  onClearCooldown: (id: string) => void;
  onRelogin: (id: string) => void;
  onRefreshQuota: (id: string) => void;
}) {
  const signedIn = Boolean(account.accessToken);
  const quota: AccountQuota | null | undefined = account.quota;
  const used = quota?.usedPercent ?? null;
  const remaining =
    quota?.remainingPercent ?? (used == null ? null : Math.max(0, 100 - used));
  const usedClamped = Math.max(0, Math.min(100, used ?? 0));
  const hasQuota = signedIn && quota != null && used != null;
  const name = account.email || account.name;

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
    <Card className="overflow-hidden">
      <CardContent className="p-0">
        <div className="grid grid-cols-1 gap-2 px-3 py-2.5 sm:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_auto] sm:items-center sm:gap-3">
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
                {quota?.lastError ? (
                  <Tag title={`${labels.error}: ${quota.lastError}`} variant="danger">
                    Err
                  </Tag>
                ) : null}
              </div>
            </div>
          </div>

          {/* Quota — always same block height for layout consistency */}
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
              {hasQuota ? (
                <>
                  <Tag
                    title={`${labels.remaining} ${formatPercent(remaining)} · ${labels.used} ${formatPercent(used)}`}
                    variant={remainingTone(used)}
                  >
                    {formatPercent(remaining)}
                  </Tag>
                  <Tag
                    title={`${labels.resetsAt}: ${formatFullDateTime(quota.resetsAt, locale)}${
                      quota.periodStartAt
                        ? ` · ${labels.periodStart}: ${formatFullDateTime(quota.periodStartAt, locale)}`
                        : ""
                    }${
                      quota.fetchedAt
                        ? ` · ${labels.fetchedAt}: ${formatFullDateTime(quota.fetchedAt, locale)}`
                        : ""
                    }`}
                  >
                    ↻ {formatShortDateTime(quota.resetsAt, locale)}
                  </Tag>
                  {productTags.map((p) => (
                    <Tag
                      key={`${p.productId}-${p.label}`}
                      title={`${p.label}: ${labels.used} ${formatPercent(p.usedPercent)}`}
                    >
                      {p.label} {formatPercent(p.usedPercent)}
                    </Tag>
                  ))}
                  {account.rateLimitRemaining != null || account.rateLimitLimit != null ? (
                    <Tag
                      title={`${labels.rateLimit}: ${
                        account.rateLimitRemaining != null && account.rateLimitLimit != null
                          ? `${account.rateLimitRemaining}/${account.rateLimitLimit}`
                          : String(account.rateLimitRemaining ?? "—")
                      }${
                        account.rateLimitResetAt
                          ? ` · ${formatFullDateTime(account.rateLimitResetAt, locale)}`
                          : ""
                      }`}
                    >
                      RL{" "}
                      {account.rateLimitRemaining != null && account.rateLimitLimit != null
                        ? `${account.rateLimitRemaining}/${account.rateLimitLimit}`
                        : String(account.rateLimitRemaining ?? "—")}
                    </Tag>
                  ) : null}
                </>
              ) : (
                <Tag title={signedIn ? labels.unknown : labels.needLogin}>—</Tag>
              )}
            </div>
          </div>

          {/* Controls — fixed action set */}
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
              // Keep action rail width stable when not in cooldown.
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
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const waitingAccountId = useRef<string | null>(null);
  const autoQuotaRef = useRef(false);

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
      setAccounts(await api.refreshAccountQuota(id));
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

  const dateLocale = locale === "en" ? "en-US" : "zh-CN";
  const hasSignedIn = accounts.some((a) => a.accessToken);

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
    rateLimit: t.accounts.rateLimitHint,
    refreshQuota: t.accounts.refreshQuota,
    refreshingQuota: t.accounts.refreshingQuota,
    relogin: t.accounts.relogin,
    loggingIn: t.accounts.loggingIn,
    clearCooldown: t.accounts.clearCooldown,
    remove: t.common.remove,
  };

  return (
    <div className="space-y-4 overflow-y-auto">
      <div className="flex flex-wrap items-start justify-between gap-3">
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
          <Button size="sm" disabled={busy} onClick={() => login()}>
            {busy ? t.accounts.loggingIn : t.accounts.addAccount}
          </Button>
        </div>
      </div>

      {error ? (
        <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 whitespace-pre-wrap">
          {error}
        </div>
      ) : null}
      {authUrl ? (
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" onClick={openAuthLink}>
            {t.accounts.openAuthLink}
          </Button>
          <Button size="sm" variant="outline" onClick={() => refresh()}>
            {t.accounts.refreshStatus}
          </Button>
        </div>
      ) : null}

      <div className="space-y-2">
        {accounts.map((account) => (
          <AccountCard
            key={account.id}
            account={account}
            locale={dateLocale}
            busy={busy}
            quotaBusy={quotaBusyAll || quotaBusyId === account.id}
            labels={cardLabels}
            onSave={(a) => void save(a)}
            onRemove={(id) => void remove(id)}
            onClearCooldown={(id) => void clearCooldown(id)}
            onRelogin={(id) => void login({ accountId: id })}
            onRefreshQuota={(id) => void refreshQuota(id)}
          />
        ))}
        {accounts.length === 0 ? (
          <div className="text-sm text-neutral-500">{t.accounts.empty}</div>
        ) : null}
      </div>
    </div>
  );
}
