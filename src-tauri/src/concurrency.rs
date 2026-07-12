//! Soft per-account in-flight request counters.
//!
//! Used only as a **routing preference** (skip overloaded accounts when others
//! are free). Never blocks a request if every account is at capacity — UX first.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTS: Lazy<Mutex<HashMap<String, AtomicU32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn in_flight(account_id: &str) -> u32 {
    let map = COUNTS.lock();
    map.get(account_id)
        .map(|c| c.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// RAII permit: increments on create, decrements on drop.
pub struct AccountPermit {
    account_id: String,
}

impl AccountPermit {
    pub fn acquire(account_id: &str) -> Self {
        {
            let mut map = COUNTS.lock();
            map.entry(account_id.to_string())
                .or_insert_with(|| AtomicU32::new(0))
                .fetch_add(1, Ordering::Relaxed);
        }
        Self {
            account_id: account_id.to_string(),
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }
}

impl Drop for AccountPermit {
    fn drop(&mut self) {
        let map = COUNTS.lock();
        if let Some(c) = map.get(&self.account_id) {
            // Saturating sub via compare — avoid underflow.
            let mut cur = c.load(Ordering::Relaxed);
            loop {
                let next = cur.saturating_sub(1);
                match c.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => break,
                    Err(v) => cur = v,
                }
            }
        }
    }
}

/// True when account is at/over soft max (max==0 means unlimited).
pub fn is_saturated(account_id: &str, max: u32) -> bool {
    if max == 0 {
        return false;
    }
    in_flight(account_id) >= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permit_inc_dec() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        assert_eq!(in_flight(&id), 0);
        {
            let _p = AccountPermit::acquire(&id);
            assert_eq!(in_flight(&id), 1);
            let _p2 = AccountPermit::acquire(&id);
            assert_eq!(in_flight(&id), 2);
            assert!(is_saturated(&id, 2));
            assert!(!is_saturated(&id, 3));
        }
        assert_eq!(in_flight(&id), 0);
    }
}
