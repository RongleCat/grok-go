use chrono::Utc;
use rand::Rng;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::concurrency;
use crate::config::{Account, AccountHealth, AppConfig, AuthStore, RoutingStrategy, load_auth, save_auth};
use crate::error::{AppError, AppResult};
use crate::session_affinity;

static RR_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Result of account selection (for logs / diagnostics).
#[derive(Debug, Clone)]
pub struct PickDecision {
    pub account: Account,
    /// sticky | fill-first | weighted-round-robin | least-recently-used | lowest-error-rate
    pub layer: &'static str,
    pub sticky_hit: bool,
    pub session_key: Option<String>,
}

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
    Ok(pick_account_decision(config, store, &[], None)?.account)
}

/// Pick a routable account, skipping any id in `exclude` (same-request failover).
pub fn pick_account_excluding(
    config: &AppConfig,
    store: &AuthStore,
    exclude: &[String],
) -> AppResult<Account> {
    Ok(pick_account_decision(config, store, exclude, None)?.account)
}

/// Full pick with optional session key for sticky routing.
pub fn pick_account_decision(
    config: &AppConfig,
    store: &AuthStore,
    exclude: &[String],
    session_key: Option<&str>,
) -> AppResult<PickDecision> {
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
            AccountHealth::Cooldown => a.cooldown_until.map(|t| t <= now).unwrap_or(true),
            _ => true,
        })
        .collect();

    if candidates.is_empty() {
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

    // Soft concurrency filter: prefer unsaturated accounts when others are free.
    let max_conc = config.account_max_concurrency;
    if max_conc > 0 {
        let unsaturated: Vec<&Account> = candidates
            .iter()
            .copied()
            .filter(|a| !concurrency::is_saturated(&a.id, max_conc))
            .collect();
        if !unsaturated.is_empty() {
            candidates = unsaturated;
        }
    }

    // Session affinity (before health narrowing so sticky degraded can still win
    // when it is the only bound account — unless excluded/cooldown).
    if config.session_affinity {
        if let Some(key) = session_key.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(bound_id) = session_affinity::lookup(key) {
                if let Some(acc) = candidates.iter().find(|a| a.id == bound_id) {
                    tracing::debug!(
                        account = %acc.id,
                        session = %key,
                        "routing sticky hit"
                    );
                    return Ok(PickDecision {
                        account: (*acc).clone(),
                        layer: "sticky",
                        sticky_hit: true,
                        session_key: Some(key.to_string()),
                    });
                }
            }
        }
    }

    // Prefer healthy / low-failure accounts when available.
    let preferred: Vec<&Account> = candidates
        .iter()
        .copied()
        .filter(|a| a.health == AccountHealth::Healthy && a.consecutive_failures == 0)
        .collect();
    if !preferred.is_empty() {
        candidates = preferred;
    } else {
        candidates.sort_by_key(|a| a.consecutive_failures);
    }

    // Optional use-it-or-lose-it: among equals, prefer soonest weekly reset.
    if config.prefer_soonest_reset {
        candidates = prefer_soonest_reset(candidates);
    }

    let (account, layer) = match config.routing_strategy {
        RoutingStrategy::FillFirst => {
            let picked = pick_fill_first(&candidates);
            (picked, "fill-first")
        }
        RoutingStrategy::WeightedRoundRobin => {
            let picked = pick_weighted_round_robin(&candidates, config.quota_aware_routing);
            (picked, "weighted-round-robin")
        }
        RoutingStrategy::LeastRecentlyUsed => {
            let mut sorted = candidates.clone();
            sorted.sort_by_key(|a| a.last_success_at);
            (sorted[0].clone(), "least-recently-used")
        }
        RoutingStrategy::LowestErrorRate => {
            let mut sorted = candidates.clone();
            sorted.sort_by_key(|a| a.consecutive_failures);
            let best = sorted[0].consecutive_failures;
            let top: Vec<_> = sorted
                .into_iter()
                .filter(|a| a.consecutive_failures == best)
                .collect();
            let idx = rand::thread_rng().gen_range(0..top.len());
            (top[idx].clone(), "lowest-error-rate")
        }
    };

    Ok(PickDecision {
        account,
        layer,
        sticky_hit: false,
        session_key: session_key.map(|s| s.to_string()),
    })
}

/// Fill-first: highest weight first (stable by id), drain primary before backups.
fn pick_fill_first(candidates: &[&Account]) -> Account {
    let mut sorted: Vec<&Account> = candidates.to_vec();
    sorted.sort_by(|a, b| {
        b.weight
            .max(1)
            .cmp(&a.weight.max(1))
            .then_with(|| a.id.cmp(&b.id))
    });
    sorted[0].clone()
}

fn pick_weighted_round_robin(candidates: &[&Account], quota_aware: bool) -> Account {
    let weights: Vec<u32> = candidates
        .iter()
        .map(|a| effective_weight(a, quota_aware))
        .collect();
    let total_weight: u32 = weights.iter().sum();
    let mut ticket = (RR_COUNTER.fetch_add(1, Ordering::Relaxed) as u32) % total_weight.max(1);
    for (account, w) in candidates.iter().zip(weights.iter()) {
        if ticket < *w {
            return (*account).clone();
        }
        ticket -= *w;
    }
    candidates[0].clone()
}

/// Base account weight × soft headroom factor from SuperGrok remaining / rate-limit.
///
/// Weights are scaled ×10 so fractional headroom (0.1–0.5) still differentiates
/// accounts that share the same UI weight of 1.
fn effective_weight(account: &Account, quota_aware: bool) -> u32 {
    let base = account.weight.max(1).saturating_mul(10);
    if !quota_aware {
        return base;
    }
    let factor = headroom_factor(account);
    // Keep at least 1 so accounts never disappear entirely.
    ((base as f32) * factor).round().max(1.0) as u32
}

fn headroom_factor(account: &Account) -> f32 {
    // SuperGrok weekly remaining (preferred signal when fresh).
    if let Some(q) = account.quota.as_ref() {
        let age_ok = q
            .fetched_at
            .signed_duration_since(Utc::now())
            .num_hours()
            .abs()
            < 24;
        if age_ok && q.last_error.is_none() {
            let rem = q.remaining_percent;
            return if rem >= 40.0 {
                1.0
            } else if rem >= 20.0 {
                0.5
            } else if rem >= 5.0 {
                0.25
            } else {
                0.1
            };
        }
    }
    // Fallback: API rate-limit remaining ratio when present.
    if let (Some(rem), Some(lim)) = (account.rate_limit_remaining, account.rate_limit_limit) {
        if lim > 0 {
            let ratio = rem as f32 / lim as f32;
            return if ratio >= 0.4 {
                1.0
            } else if ratio >= 0.15 {
                0.5
            } else {
                0.2
            };
        }
    }
    1.0
}

fn prefer_soonest_reset<'a>(mut candidates: Vec<&'a Account>) -> Vec<&'a Account> {
    candidates.sort_by(|a, b| {
        let ra = a.quota.as_ref().and_then(|q| q.resets_at);
        let rb = b.quota.as_ref().and_then(|q| q.resets_at);
        match (ra, rb) {
            (Some(ta), Some(tb)) => ta.cmp(&tb),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    // Keep only the earliest reset cohort when multiple share the same timestamp.
    if let Some(first) = candidates.first().and_then(|a| a.quota.as_ref().and_then(|q| q.resets_at)) {
        let cohort: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|a| {
                a.quota
                    .as_ref()
                    .and_then(|q| q.resets_at)
                    .map(|t| t == first)
                    .unwrap_or(false)
            })
            .collect();
        if !cohort.is_empty() {
            return cohort;
        }
    }
    candidates
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
        if account.quota.is_none() {
            account.quota = slot.quota.clone();
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
    session_affinity::invalidate_account(account_id);
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
    use crate::quota::{AccountQuotaSnapshot, QuotaProductUsage};

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
        bad.weight = 100;
        let store = AuthStore {
            accounts: vec![bad, good],
        };
        let cfg = AppConfig {
            routing_strategy: RoutingStrategy::WeightedRoundRobin,
            session_affinity: false,
            ..AppConfig::default()
        };
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

    #[test]
    fn sticky_binds_session() {
        let a = logged_in("a", 0, AccountHealth::Healthy);
        let b = logged_in("b", 0, AccountHealth::Healthy);
        let a_id = a.id.clone();
        let store = AuthStore {
            accounts: vec![a, b],
        };
        let cfg = AppConfig {
            session_affinity: true,
            session_affinity_ttl_secs: 3600,
            ..AppConfig::default()
        };
        session_affinity::bind("sess-test-1", &a_id, 3600);
        let d = pick_account_decision(&cfg, &store, &[], Some("sess-test-1")).unwrap();
        assert!(d.sticky_hit);
        assert_eq!(d.account.id, a_id);
        assert_eq!(d.layer, "sticky");
        session_affinity::invalidate("sess-test-1");
    }

    #[test]
    fn fill_first_picks_highest_weight() {
        let mut low = logged_in("low", 0, AccountHealth::Healthy);
        let mut high = logged_in("high", 0, AccountHealth::Healthy);
        low.weight = 1;
        high.weight = 10;
        let high_id = high.id.clone();
        let store = AuthStore {
            accounts: vec![low, high],
        };
        let cfg = AppConfig {
            routing_strategy: RoutingStrategy::FillFirst,
            session_affinity: false,
            quota_aware_routing: false,
            ..AppConfig::default()
        };
        for _ in 0..5 {
            let d = pick_account_decision(&cfg, &store, &[], None).unwrap();
            assert_eq!(d.account.id, high_id);
            assert_eq!(d.layer, "fill-first");
        }
    }

    #[test]
    fn quota_aware_downweights_low_remaining() {
        let mut rich = logged_in("rich", 0, AccountHealth::Healthy);
        let mut poor = logged_in("poor", 0, AccountHealth::Healthy);
        rich.weight = 1;
        poor.weight = 1;
        rich.quota = Some(AccountQuotaSnapshot::from_used(
            10.0,
            None,
            None,
            Vec::<QuotaProductUsage>::new(),
        ));
        poor.quota = Some(AccountQuotaSnapshot::from_used(
            95.0,
            None,
            None,
            Vec::<QuotaProductUsage>::new(),
        ));
        assert!(effective_weight(&rich, true) > effective_weight(&poor, true));
        assert_eq!(effective_weight(&poor, false), 10); // weight 1 × scale 10
    }
}
