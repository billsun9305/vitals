//! One prior observation, for trend detection in `pressure`.
//!
//! Deliberately not a ring buffer: a ring shared by concurrent CLI
//! invocations needs locking, and the only trend `vitals` reports is a delta
//! against a single earlier point. One atomically-replaced file has no
//! locking, no partial-write window, and no stale-index failure mode.
//!
//! Only CLI invocations write this file. The tray never does — that would
//! be cross-process coupling by the back door.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Older than this and the observation is not usable as a trend baseline.
pub const MAX_AGE_S: u64 = 900;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Observation {
    pub schema_version: u32,
    pub unix_s: u64,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
    pub swap_used_mb: u64,
}

pub fn unix_now_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `$XDG_CACHE_HOME/vitals/last.json`, else `~/.cache/vitals/last.json`.
pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("vitals").join("last.json")
}

/// Load the stored observation, or `None` if it is absent, unreadable,
/// corrupt, from a different schema, stale, or in the future.
pub fn load_from(p: &Path, now_s: u64) -> Option<Observation> {
    let bytes = std::fs::read(p).ok()?;
    let obs: Observation = serde_json::from_slice(&bytes).ok()?;
    if obs.schema_version != crate::schema::SCHEMA_VERSION {
        return None;
    }
    if obs.unix_s > now_s {
        return None; // clock moved backwards
    }
    if now_s - obs.unix_s > MAX_AGE_S {
        return None;
    }
    Some(obs)
}

/// Write via a sibling temp file and `rename`, so a reader never sees a
/// half-written document.
pub fn store_to(p: &Path, obs: &Observation) -> std::io::Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(obs)?)?;
    std::fs::rename(&tmp, p)
}

pub fn load(now_s: u64) -> Option<Observation> {
    load_from(&path(), now_s)
}

pub fn store(obs: &Observation) -> std::io::Result<()> {
    store_to(&path(), obs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vitals-test-{}-{}.json", std::process::id(), tag))
    }

    fn obs(unix_s: u64, swap: u64) -> Observation {
        Observation {
            schema_version: 1,
            unix_s,
            mem_used_mb: 1000,
            mem_total_mb: 36864,
            swap_used_mb: swap,
        }
    }

    #[test]
    fn round_trips_through_the_file() {
        let p = temp_path("roundtrip");
        let _ = std::fs::remove_file(&p);
        store_to(&p, &obs(1_000_000, 512)).unwrap();
        let got = load_from(&p, 1_000_010).expect("should load");
        assert_eq!(got.swap_used_mb, 512);
        assert_eq!(got.unix_s, 1_000_000);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn missing_file_is_none_not_an_error() {
        assert!(load_from(&temp_path("absent"), 1_000_000).is_none());
    }

    #[test]
    fn stale_observations_are_rejected() {
        let p = temp_path("stale");
        store_to(&p, &obs(1_000_000, 512)).unwrap();
        assert!(load_from(&p, 1_000_000 + MAX_AGE_S + 1).is_none());
        assert!(load_from(&p, 1_000_000 + MAX_AGE_S - 1).is_some());
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn future_timestamps_are_rejected() {
        // A clock change must not produce a negative elapsed time.
        let p = temp_path("future");
        store_to(&p, &obs(2_000_000, 512)).unwrap();
        assert!(load_from(&p, 1_000_000).is_none());
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn mismatched_schema_version_is_rejected() {
        let p = temp_path("schema-mismatch");
        let mut bad = obs(1_000_000, 512);
        bad.schema_version = 999;
        store_to(&p, &bad).unwrap();
        assert!(load_from(&p, 1_000_010).is_none());
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn corrupt_files_are_ignored() {
        let p = temp_path("corrupt");
        std::fs::write(&p, b"{ not json").unwrap();
        assert!(load_from(&p, 1_000_000).is_none());
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn store_replaces_atomically_leaving_no_tmp_file() {
        let p = temp_path("atomic");
        store_to(&p, &obs(1_000_000, 1)).unwrap();
        store_to(&p, &obs(1_000_001, 2)).unwrap();
        assert_eq!(load_from(&p, 1_000_002).unwrap().swap_used_mb, 2);
        assert!(!p.with_extension("json.tmp").exists());
        std::fs::remove_file(&p).unwrap();
    }
}
