import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type Account } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { useI18n } from "@/i18n/context";
import { useToast } from "@/components/ui/toast";

export function AccountsPage() {
  const { t } = useI18n();
  const { toast } = useToast();
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [error, setError] = useState("");
  const [authUrl, setAuthUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const waitingAccountId = useRef<string | null>(null);

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
    refresh();
    return () => stopPoll();
  }, [refresh, stopPoll]);

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
      // Backend already tried to open the browser; only fall back if it failed
      // (otherwise two authorize tabs open on one click).
      let opened = start.browserOpened;
      if (!opened) {
        try {
          await openUrl(start.authorizeUrl);
          opened = true;
        } catch {
          // ignore — user can click “Open auth page”
        }
      }
      toast(opened ? t.accounts.browserOpened : t.accounts.browserOpenFailed, opened ? "success" : "warning");
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

  function healthLabel(health: Account["health"]) {
    return t.accounts.health[health] ?? health;
  }

  function displayName(account: Account) {
    return account.email || account.name;
  }

  return (
    <div className="space-y-4 overflow-y-auto">
      <div className="flex items-center justify-between gap-3">
        <h1 className="text-xl font-semibold tracking-tight">{t.accounts.title}</h1>
        <Button disabled={busy} onClick={() => login()}>
          {busy ? t.accounts.loggingIn : t.accounts.addAccount}
        </Button>
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

      <div className="space-y-3">
        {accounts.map((account) => (
          <Card key={account.id}>
            <CardContent className="flex flex-col gap-4 p-4 md:flex-row md:items-center md:justify-between">
              <div className="min-w-0 space-y-1">
                <div className="flex flex-wrap items-center gap-2">
                  <div className="truncate font-medium">{displayName(account)}</div>
                  <Badge
                    variant={
                      account.health === "healthy"
                        ? "success"
                        : account.health === "degraded"
                          ? "warning"
                          : "danger"
                    }
                  >
                    {healthLabel(account.health)}
                  </Badge>
                </div>
                <div className="text-xs text-neutral-500">
                  {account.accessToken ? t.accounts.loggedIn : t.accounts.notLoggedIn}
                </div>
              </div>
              <div className="flex flex-wrap items-center gap-3">
                <div className="flex items-center gap-2 text-sm">
                  <span>{t.common.enabled}</span>
                  <Switch
                    checked={account.enabled}
                    onCheckedChange={(v) => save({ ...account, enabled: v })}
                  />
                </div>
                <div className="flex items-center gap-2 text-sm">
                  <span>{t.common.weight}</span>
                  <Input
                    className="w-20"
                    type="number"
                    min={1}
                    value={account.weight}
                    onChange={(e) =>
                      save({ ...account, weight: Math.max(1, Number(e.target.value) || 1) })
                    }
                  />
                </div>
                {account.health === "cooldown" ? (
                  <Button variant="outline" onClick={() => clearCooldown(account.id)}>
                    {t.accounts.clearCooldown}
                  </Button>
                ) : null}
                <Button
                  variant="outline"
                  disabled={busy}
                  onClick={() => login({ accountId: account.id })}
                >
                  {busy ? t.accounts.loggingIn : t.accounts.relogin}
                </Button>
                <Button variant="destructive" disabled={busy} onClick={() => remove(account.id)}>
                  {t.common.remove}
                </Button>
              </div>
            </CardContent>
          </Card>
        ))}
        {accounts.length === 0 ? (
          <div className="text-sm text-neutral-500">{t.accounts.empty}</div>
        ) : null}
      </div>
    </div>
  );
}
