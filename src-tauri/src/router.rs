use chrono::Utc;
use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::{Account, AccountHealth, AppConfig, AuthStore, RoutingStrategy, load_auth, save_auth};
use crate::error::{AppError, AppResult};

static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub fn list_accounts() -> AppResult<Vec<Account>> {
    let mut store = load_auth()?;
    if crate::auth::clear_expired_cooldowns_in_store(&mut store) {
        save_auth(&store)?;
    }
    Ok(store.accounts)
}

pub fn save_accounts(accounts: Vec<Account>) -> AppResult<()> {
    save_auth(&AuthStore { accounts })
}

pub fn pick_account(config: &AppConfig, store: &AuthStore) -> AppResult<Account> {
    pick_account_excluding(config, store, &[])
}

/// Pick a routable account, skipping any id in `exclude` (same-request failover).
pub fn pick_account_excluding(
    config: &AppConfig,
    store: &AuthStore,
    exclude: &[String],
) -> AppResult<Account> {
    // Expire local cooldowns so routing/UI don't stick on stale "cooldown".
    let mut store_owned = store.clone();
    if crate::auth::clear_expired_cooldowns_in_store(&mut store_owned) {
        let _ = save_auth(&store_owned);
    }
    let store = &store_owned;

    let now = Utc::now();
    let logged_in: Vec<&Account> = store
        .accounts
        .iter()
        .filter(|a| a.enabled)
        .filter(|a| a.access_token.is_some() || a.refresh_token.is_some())
        .filter(|a| !exclude.iter().any(|id| id == &a.id))
        .collect();

    let mut candidates: Vec<&Account> = logged_in
        .iter()
        .copied()
        .filter(|a| match a.health {
            AccountHealth::Disabled => false,
            // Still in cooldown window → skip; expired ones were cleared above.
            AccountHealth::Cooldown => a.cooldown_until.map(|t| t <= now).unwrap_or(true),
            _ => true,
        })
        .collect();

    if candidates.is_empty() {
        // Distinguish "all excluded this request" vs "none available at all".
        if !exclude.is_empty() {
            return Err(AppError::msg(
                "no more accounts available for failover on this request",
            ));
        }
        let all_logged_in: Vec<&Account> = store
            .accounts
            .iter()
            .filter(|a| a.enabled)
            .filter(|a| a.access_token.is_some() || a.refresh_token.is_some())
            .collect();
        if all_logged_in.is_empty() {
            return Err(AppError::msg(
                "no logged-in accounts available; open Accounts and complete xAI OAuth",
            ));
        }
        let until = all_logged_in
            .iter()
            .filter_map(|a| a.cooldown_until)
            .min()
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| "unknown".into());
        return Err(AppError::msg(format!(
            "all accounts are in local rate-limit cooldown until {until} \
             (local heuristic / 429 — not permanent). Wait or clear cooldown in Accounts."
        )));
    }

    // Prefer healthy / low-failure accounts when available so a degraded 403 account
    // does not keep winning WRR while others are fine.
    let preferred: Vec<&Account> = candidates
        .iter()
        .copied()
        .filter(|a| {
            a.health == AccountHealth::Healthy && a.consecutive_failures == 0
        })
        .collect();
    if !preferred.is_empty() {
        candidates = preferred;
    } else {
        // Soft preference: lower consecutive failures first for non-LRU strategies.
        candidates.sort_by_key(|a| a.consecutive_failures);
    }

    match config.routing_strategy {
        RoutingStrategy::WeightedRoundRobin => {
            let total_weight: u32 = candidates.iter().map(|a| a.weight.max(1)).sum();
            let mut ticket =
                (RR_COUNTER.fetch_add(1, Ordering::Relaxed) as u32) % total_weight.max(1);
            for account in &candidates {
                let w = account.weight.max(1);
                if ticket < w {
                    return Ok((*account).clone());
                }
                ticket -= w;
            }
            Ok(candidates[0].clone())
        }
        RoutingStrategy::LeastRecentlyUsed => {
            candidates.sort_by_key(|a| a.last_success_at);
            Ok(candidates[0].clone())
        }
        RoutingStrategy::LowestErrorRate => {
            // already sorted by consecutive_failures above when no healthy preferred set
            candidates.sort_by_key(|a| a.consecutive_failures);
            let best = candidates[0].consecutive_failures;
            let top: Vec<_> = candidates
                .into_iter()
                .filter(|a| a.consecutive_failures == best)
                .collect();
            let idx = rand::thread_rng().gen_range(0..top.len());
            Ok(top[idx].clone())
        }
    }
}

/// How many enabled+logged-in accounts are currently routable (not cooldown).
pub fn routable_account_count(store: &AuthStore) -> usize {
    let now = Utc::now();
    store
        .accounts
        .iter()
        .filter(|a| a.enabled)
        .filter(|a| a.access_token.is_some() || a.refresh_token.is_some())
        .filter(|a| match a.health {
            AccountHealth::Disabled => false,
            AccountHealth::Cooldown => a.cooldown_until.map(|t| t <= now).unwrap_or(true),
            _ => true,
        })
        .count()
}

pub fn update_account(mut account: Account) -> AppResult<()> {
    let mut store = load_auth()?;
    if let Some(slot) = store.accounts.iter_mut().find(|a| a.id == account.id) {
        // preserve secrets / telemetry if UI omitted them
        if account.access_token.is_none() {
            account.access_token = slot.access_token.clone();
        }
        if account.refresh_token.is_none() {
            account.refresh_token = slot.refresh_token.clone();
        }
        if account.email.is_none() {
            account.email = slot.email.clone();
        }
        if account.rate_limit_limit.is_none() {
            account.rate_limit_limit = slot.rate_limit_limit;
        }
        if account.rate_limit_remaining.is_none() {
            account.rate_limit_remaining = slot.rate_limit_remaining;
        }
        if account.rate_limit_reset_at.is_none() {
            account.rate_limit_reset_at = slot.rate_limit_reset_at;
        }
        if account.last_upstream_error.is_none() {
            account.last_upstream_error = slot.last_upstream_error.clone();
        }
        *slot = account;
        save_auth(&store)?;
        Ok(())
    } else {
        Err(AppError::msg("account not found"))
    }
}

pub fn remove_account(account_id: &str) -> AppResult<()> {
    let mut store = load_auth()?;
    store.accounts.retain(|a| a.id != account_id);
    save_auth(&store)
}

/// Persist account tokens/health to disk (and refresh cache).
pub fn replace_account_tokens(account: &Account) -> AppResult<()> {
    let mut store = load_auth()?;
    if let Some(slot) = store.accounts.iter_mut().find(|a| a.id == account.id) {
        *slot = account.clone();
        save_auth(&store)?;
        Ok(())
    } else {
        Err(AppError::msg("account not found"))
    }
}

/// Memory-only account update for the success hot path (avoids auth.json rewrite every request).
pub fn touch_account_cache(account: &Account) {
    crate::config::patch_account_cache(account);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Account, AccountHealth, AppConfig, AuthStore};

    fn logged_in(name: &str, failures: u32, health: AccountHealth) -> Account {
        let mut a = Account::new(name);
        a.access_token = Some(format!("tok-{name}"));
        a.consecutive_failures = failures;
        a.health = health;
        a
    }

    #[test]
    fn pick_excluding_skips_ids() {
        let a = logged_in("a", 0, AccountHealth::Healthy);
        let b = logged_in("b", 0, AccountHealth::Healthy);
        let id_a = a.id.clone();
        let store = AuthStore {
            accounts: vec![a, b.clone()],
        };
        let cfg = AppConfig::default();
        let picked = pick_account_excluding(&cfg, &store, &[id_a]).unwrap();
        assert_eq!(picked.id, b.id);
    }

    #[test]
    fn pick_prefers_healthy_zero_failure() {
        let mut bad = logged_in("bad", 5, AccountHealth::Degraded);
        let good = logged_in("good", 0, AccountHealth::Healthy);
        let good_id = good.id.clone();
        // Give bad higher weight so WRR would prefer it without health preference.
        bad.weight = 100;
        let store = AuthStore {
            accounts: vec![bad, good],
        };
        let cfg = AppConfig {
            routing_strategy: RoutingStrategy::WeightedRoundRobin,
            ..AppConfig::default()
        };
        // Multiple picks should stick to healthy when available.
        for _ in 0..8 {
            let picked = pick_account(&cfg, &store).unwrap();
            assert_eq!(picked.id, good_id, "should prefer healthy account");
        }
    }

    #[test]
    fn pick_skips_cooldown() {
        let mut cooled = logged_in("cooled", 1, AccountHealth::Cooldown);
        cooled.cooldown_until = Some(Utc::now() + chrono::Duration::seconds(120));
        let ok = logged_in("ok", 0, AccountHealth::Healthy);
        let ok_id = ok.id.clone();
        let store = AuthStore {
            accounts: vec![cooled, ok],
        };
        let picked = pick_account(&AppConfig::default(), &store).unwrap();
        assert_eq!(picked.id, ok_id);
    }

    #[test]
    fn routable_count_ignores_cooldown() {
        let mut cooled = logged_in("cooled", 1, AccountHealth::Cooldown);
        cooled.cooldown_until = Some(Utc::now() + chrono::Duration::seconds(120));
        let ok = logged_in("ok", 0, AccountHealth::Healthy);
        let store = AuthStore {
            accounts: vec![cooled, ok],
        };
        assert_eq!(routable_account_count(&store), 1);
    }
}
