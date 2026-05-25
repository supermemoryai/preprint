use std::path::PathBuf;

use serde_json::Value;

#[cfg(target_os = "macos")]
fn chrome_user_data_dir() -> Option<PathBuf> {
    let base = dirs::home_dir()?.join("Library/Application Support");
    for name in [
        "Google/Chrome",
        "Google/Chrome Canary",
        "Chromium",
        "BraveSoftware/Brave-Browser",
    ] {
        let dir = base.join(name);
        if dir.join("Local State").is_file() {
            return Some(dir);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn chrome_user_data_dir() -> Option<PathBuf> {
    let base = dirs::home_dir()?.join(".config");
    for name in [
        "google-chrome",
        "google-chrome-unstable",
        "chromium",
        "BraveSoftware/Brave-Browser",
    ] {
        let dir = base.join(name);
        if dir.join("Local State").is_file() {
            return Some(dir);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn chrome_user_data_dir() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let base = PathBuf::from(local);
    for name in [
        r"Google\Chrome\User Data",
        r"Google\Chrome SxS\User Data",
        r"Chromium\User Data",
        r"BraveSoftware\Brave-Browser\User Data",
    ] {
        let dir = base.join(name);
        if dir.join("Local State").is_file() {
            return Some(dir);
        }
    }
    None
}

fn load_local_state() -> Option<Value> {
    let path = chrome_user_data_dir()?.join("Local State");
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn detect_default_profile_dir() -> Option<String> {
    let state = load_local_state()?;
    if let Some(last_used) = state
        .get("profile")
        .and_then(|p| p.get("last_used"))
        .and_then(|v| v.as_str())
    {
        if !last_used.is_empty() {
            return Some(last_used.to_string());
        }
    }
    list_profile_dirs()
        .into_iter()
        .next()
        .or_else(|| Some("Default".to_string()))
}

pub fn list_profile_dirs() -> Vec<String> {
    let Some(state) = load_local_state() else {
        return Vec::new();
    };
    let Some(info_cache) = state
        .get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
    else {
        return Vec::new();
    };
    let mut dirs: Vec<String> = info_cache.keys().cloned().collect();
    dirs.sort();
    dirs
}

#[derive(Debug, Clone)]
struct ProfileEntry {
    directory: String,
    display: String,
}

fn list_profile_entries() -> Vec<ProfileEntry> {
    let Some(state) = load_local_state() else {
        return Vec::new();
    };
    let Some(info_cache) = state
        .get("profile")
        .and_then(|p| p.get("info_cache"))
        .and_then(|c| c.as_object())
    else {
        return Vec::new();
    };
    info_cache
        .iter()
        .map(|(dir, info)| {
            let display = info
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(dir)
                .to_string();
            ProfileEntry {
                directory: dir.clone(),
                display,
            }
        })
        .collect()
}

pub fn resolve_profile_dir(input: &str) -> Option<String> {
    let entries = list_profile_entries();
    if entries.is_empty() {
        return None;
    }
    if let Some(e) = entries.iter().find(|e| e.directory == input) {
        return Some(e.directory.clone());
    }
    let lower = input.to_lowercase();
    if let Some(e) = entries
        .iter()
        .find(|e| e.display.to_lowercase() == lower)
    {
        return Some(e.directory.clone());
    }
    if let Some(e) = entries
        .iter()
        .find(|e| e.directory.to_lowercase() == lower)
    {
        return Some(e.directory.clone());
    }
    None
}

pub fn slug_for_session(profile_name: &str) -> String {
    let mut out = String::with_capacity(profile_name.len());
    let mut prev_dash = false;
    for ch in profile_name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "profile".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basics() {
        assert_eq!(slug_for_session("Work"), "work");
        assert_eq!(slug_for_session("Profile 1"), "profile-1");
        assert_eq!(slug_for_session("My Side Project"), "my-side-project");
        assert_eq!(slug_for_session("Default"), "default");
        assert_eq!(slug_for_session("PERSON_ONE"), "person-one");
    }

    #[test]
    fn slug_collapses_runs() {
        assert_eq!(slug_for_session("a   b"), "a-b");
        assert_eq!(slug_for_session("foo___bar---baz"), "foo-bar-baz");
    }

    #[test]
    fn slug_handles_edges() {
        assert_eq!(slug_for_session("   "), "profile");
        assert_eq!(slug_for_session(""), "profile");
        assert_eq!(slug_for_session("trailing-"), "trailing");
    }
}
