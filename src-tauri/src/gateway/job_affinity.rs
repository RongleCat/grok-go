//! Sticky account affinity for multi-step upstream jobs (esp. deferred video).
//!
//! xAI video `request_id`s are **account-scoped**. Submitting with account A and
//! polling with account B yields HTTP 404 `Failed to read static file.`
//!
//! This module remembers which OAuth account created a job so poll / follow-up
//! steps reuse the same credential.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long we keep job → account mapping (video jobs rarely exceed a few minutes).
const JOB_TTL: Duration = Duration::from_secs(45 * 60);
/// Cap map size so a long-running process cannot grow without bound.
const MAX_ENTRIES: usize = 2048;

#[derive(Clone, Debug)]
struct JobOwner {
    account_id: String,
    created_at: Instant,
}

static VIDEO_JOBS: Lazy<Mutex<HashMap<String, JobOwner>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn prune_locked(map: &mut HashMap<String, JobOwner>) {
    let now = Instant::now();
    map.retain(|_, owner| now.duration_since(owner.created_at) < JOB_TTL);
    if map.len() > MAX_ENTRIES {
        // Drop oldest first.
        let mut entries: Vec<(String, Instant)> = map
            .iter()
            .map(|(k, v)| (k.clone(), v.created_at))
            .collect();
        entries.sort_by_key(|(_, t)| *t);
        let drop_n = map.len() - MAX_ENTRIES;
        for (k, _) in entries.into_iter().take(drop_n) {
            map.remove(&k);
        }
    }
}

/// Remember which account submitted a deferred video job.
pub fn remember_video_job(request_id: &str, account_id: &str) {
    let id = request_id.trim();
    if id.is_empty() || account_id.is_empty() {
        return;
    }
    let mut map = VIDEO_JOBS.lock();
    prune_locked(&mut map);
    map.insert(
        id.to_string(),
        JobOwner {
            account_id: account_id.to_string(),
            created_at: Instant::now(),
        },
    );
}

/// Lookup the account that submitted this video job, if still remembered.
pub fn lookup_video_job_account(request_id: &str) -> Option<String> {
    let id = request_id.trim();
    if id.is_empty() {
        return None;
    }
    let mut map = VIDEO_JOBS.lock();
    prune_locked(&mut map);
    map.get(id).map(|o| o.account_id.clone())
}

/// Extract `request_id` / `id` from a video submit JSON body.
pub fn extract_video_request_id(value: &serde_json::Value) -> Option<String> {
    value
        .get("request_id")
        .or_else(|| value.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remember_and_lookup() {
        remember_video_job("job-abc", "acct-1");
        assert_eq!(
            lookup_video_job_account("job-abc").as_deref(),
            Some("acct-1")
        );
        assert!(lookup_video_job_account("missing").is_none());
    }

    #[test]
    fn extract_id() {
        assert_eq!(
            extract_video_request_id(&json!({"request_id": "r1"})).as_deref(),
            Some("r1")
        );
        assert_eq!(
            extract_video_request_id(&json!({"id": "i1"})).as_deref(),
            Some("i1")
        );
    }
}
