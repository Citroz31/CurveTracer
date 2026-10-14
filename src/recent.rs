//! Memory of the recently created / opened workspaces (stored in the user
//! configuration folder, shared by every workspace).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 15;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecentWorkspace {
    pub path: PathBuf,
    /// Last time it was opened / saved (seconds since 1970).
    pub last: u64,
}

impl RecentWorkspace {
    pub fn name(&self) -> String {
        self.path
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }

    pub fn exists(&self) -> bool {
        self.path.is_file()
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct RecentFile {
    workspaces: Vec<RecentWorkspace>,
}

/// Folder of the application settings.
pub fn config_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| b.join("CurveTracer"))
}

/// Default folder of the new workspaces: Documents/CurveTracer.
pub fn default_workspace_dir() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let docs = home.join("Documents");
    if docs.is_dir() { docs } else { home }.join("CurveTracer")
}

fn file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("recent_workspaces.json"))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Recent workspaces, most recent first.
pub fn load() -> Vec<RecentWorkspace> {
    let Some(f) = file() else {
        return Vec::new();
    };
    let mut list = std::fs::read_to_string(f)
        .ok()
        .and_then(|t| serde_json::from_str::<RecentFile>(&t).ok())
        .map(|r| r.workspaces)
        .unwrap_or_default();
    list.sort_by_key(|r| std::cmp::Reverse(r.last));
    list
}

fn save(list: &[RecentWorkspace]) {
    let Some(f) = file() else {
        return;
    };
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let r = RecentFile {
        workspaces: list.to_vec(),
    };
    if let Ok(text) = serde_json::to_string_pretty(&r) {
        let _ = std::fs::write(f, text);
    }
}

fn same(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    a == b || canon(a) == canon(b)
}

/// Puts a workspace at the top of the list (and saves the list).
pub fn touch(list: &mut Vec<RecentWorkspace>, path: &Path) {
    list.retain(|r| !same(&r.path, path));
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    list.insert(0, RecentWorkspace { path, last: now() });
    list.truncate(MAX_RECENT);
    save(list);
}

pub fn remove(list: &mut Vec<RecentWorkspace>, path: &Path) {
    list.retain(|r| !same(&r.path, path));
    save(list);
}

pub fn clear(list: &mut Vec<RecentWorkspace>) {
    list.clear();
    save(list);
}

/// "just now", "5 min ago", "3 h ago", "2 days ago"…
pub fn ago(t: u64) -> String {
    let d = now().saturating_sub(t);
    match d {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        86_400..=172_799 => "yesterday".to_owned(),
        _ if d < 86_400 * 60 => format!("{} days ago", d / 86_400),
        _ => format!("{} months ago", d / (86_400 * 30)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time() {
        let n = now();
        assert_eq!(ago(n), "just now");
        assert_eq!(ago(n - 125), "2 min ago");
        assert_eq!(ago(n - 3 * 3600), "3 h ago");
        assert_eq!(ago(n - 90_000), "yesterday");
        assert_eq!(ago(n - 5 * 86_400), "5 days ago");
    }

    #[test]
    fn names() {
        let r = RecentWorkspace {
            path: PathBuf::from("/a/b/My project.ctw"),
            last: 0,
        };
        assert_eq!(r.name(), "My project");
        assert!(!r.exists());
    }
}
