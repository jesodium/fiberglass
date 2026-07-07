use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

const REPO: &str = "jesodium/fiberglass";
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

#[derive(Serialize, Deserialize)]
struct UpdateCache {
    tag: String,
    timestamp: u64,
    /// Release notes (GitHub release `body`), shown in the TUI update modal.
    #[serde(default)]
    body: String,
}

fn cache_path() -> Option<PathBuf> {
    crate::config::config_dir()
        .ok()
        .map(|d| d.join("update_check.json"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cache() -> Option<UpdateCache> {
    let data = fs::read_to_string(cache_path()?).ok()?;
    serde_json::from_str(&data).ok()
}

fn write_cache(tag: &str, body: &str) {
    let Some(path) = cache_path() else { return };
    let _ = fs::create_dir_all(path.parent().unwrap_or(path.as_path()));
    let cache = UpdateCache {
        tag: tag.to_string(),
        timestamp: now_secs(),
        body: body.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = fs::write(path, json);
    }
}

/// Fetch the latest release tag and its notes body from GitHub.
fn fetch_latest_release() -> Option<(String, String)> {
    let output = Command::new("curl")
        .args([
            "-sSf",
            "--max-time",
            "8",
            "-H",
            "User-Agent: fiberglass",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let body = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;
    let tag = json["tag_name"].as_str()?.to_string();
    let notes = json["body"].as_str().unwrap_or("").to_string();
    Some((tag, notes))
}

/// Release notes for the cached latest version, if any. No network call.
pub(crate) fn changelog() -> Option<String> {
    let cache = read_cache()?;
    let notes = cache.body.trim();
    if notes.is_empty() {
        None
    } else {
        Some(notes.to_string())
    }
}

fn is_newer(tag: &str) -> bool {
    let latest = tag.trim_start_matches('v');
    let current = env!("CARGO_PKG_VERSION");
    if latest == current {
        return false;
    }
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut it = s.split('.');
        let major = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(latest) > parse(current)
}

/// Check the on-disk cache for a newer version. No network call.
/// Returns `Some(tag)` if the cached latest release is newer than the running binary.
pub(crate) fn check_update() -> Option<String> {
    let cache = read_cache()?;
    if is_newer(&cache.tag) {
        Some(cache.tag)
    } else {
        None
    }
}

/// If the cache is missing or older than 24 h, spawn a background thread to refresh it.
pub(crate) fn refresh_cache_if_stale() {
    let stale = match read_cache() {
        Some(c) => now_secs().saturating_sub(c.timestamp) >= CACHE_TTL.as_secs(),
        None => true,
    };
    if stale {
        std::thread::spawn(|| {
            if let Some((tag, notes)) = fetch_latest_release() {
                write_cache(&tag, &notes);
            }
        });
    }
}
