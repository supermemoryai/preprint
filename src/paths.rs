use std::path::{Path, PathBuf};

use anyhow::Context;
use url::Url;

pub const ACTION_MARKER: &str = "<!-- preprint:actions -->";
pub const TAB_PREFIX: &str = "t";

#[derive(Debug, Clone)]
pub struct WorkspacePaths {
    root: PathBuf,
}

impl WorkspacePaths {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("failed to resolve workspace {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn page_dir(&self) -> PathBuf {
        self.root.join("preprint")
    }

    pub fn meta_dir(&self) -> PathBuf {
        self.root.join(".preprint")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.meta_dir().join("artifacts")
    }

    pub fn daemon_pid(&self) -> PathBuf {
        self.meta_dir().join("daemon.pid")
    }

    pub fn daemon_log(&self) -> PathBuf {
        self.meta_dir().join("daemon.log")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.meta_dir().join("logs")
    }

    pub fn ensure(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(self.page_dir()).with_context(|| {
            format!(
                "failed to create preprint files directory {}",
                self.page_dir().display()
            )
        })?;
        std::fs::create_dir_all(self.artifacts_dir()).with_context(|| {
            format!(
                "failed to create preprint artifacts directory {}",
                self.artifacts_dir().display()
            )
        })?;
        Ok(())
    }

    pub fn page_file(&self, tab_key: &str) -> PathBuf {
        self.page_dir().join(format!("{tab_key}.md"))
    }

    pub fn diff_file(&self, tab_key: &str) -> PathBuf {
        self.page_dir().join(format!("{tab_key}.diff.md"))
    }

    pub fn tab_artifacts_dir(&self, tab_key: &str) -> PathBuf {
        self.artifacts_dir().join(tab_key)
    }

    pub fn session_file(&self, tab_key: &str) -> PathBuf {
        self.tab_artifacts_dir(tab_key).join("session.json")
    }

    pub fn screenshots_dir(&self, tab_key: &str) -> PathBuf {
        self.tab_artifacts_dir(tab_key).join("screenshots")
    }

    pub fn screenshot_file(&self, tab_key: &str, name: &str) -> PathBuf {
        self.screenshots_dir(tab_key).join(format!("{name}.png"))
    }

    pub fn tab_console_log(&self, tab_key: &str) -> PathBuf {
        self.tab_artifacts_dir(tab_key).join("console.md")
    }

    pub fn tab_errors_cursor(&self, tab_key: &str) -> PathBuf {
        self.tab_artifacts_dir(tab_key).join(".errors_cursor")
    }

    pub fn recordings_dir(&self, tab_key: &str) -> PathBuf {
        self.tab_artifacts_dir(tab_key).join("recordings")
    }

    pub fn recording_file(&self, tab_key: &str, name: &str) -> PathBuf {
        self.recordings_dir(tab_key).join(format!("{name}.webm"))
    }

    pub fn tabs_manifest(&self) -> PathBuf {
        self.page_dir().join("tabs.md")
    }

    pub fn skill_md(&self) -> PathBuf {
        self.root
            .join(".claude")
            .join("skills")
            .join("preprint-browser")
            .join("SKILL.md")
    }
}

pub fn site_from_url(raw_url: &str) -> anyhow::Result<String> {
    let url = Url::parse(raw_url).with_context(|| format!("invalid url: {raw_url}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("url has no host: {raw_url}"))?;
    Ok(sanitize_site(host))
}

pub fn sanitize_site(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub fn tab_key(initial_host: &str, tab_id: &str, session_name: &str) -> String {
    let id = if tab_id.starts_with(TAB_PREFIX) {
        format!("{initial_host}-{tab_id}")
    } else {
        format!("{initial_host}-{TAB_PREFIX}{tab_id}")
    };
    format!("{id}.{session_name}")
}

pub fn short_hash(input: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let hash = hasher.finalize();
    format!("{hash:x}").chars().take(16).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_tab_key_from_host_tab_id_and_session() {
        assert_eq!(tab_key("gmail.com", "t2", "default"), "gmail.com-t2.default");
        assert_eq!(tab_key("gmail.com", "2", "default"), "gmail.com-t2.default");
        assert_eq!(tab_key("gmail.com", "t1", "work"), "gmail.com-t1.work");
        assert_eq!(tab_key("github.com", "t3", "no-profile"), "github.com-t3.no-profile");
    }

    #[test]
    fn runtime_state_lives_inside_workspace() {
        let paths = WorkspacePaths {
            root: PathBuf::from("/tmp/workspace"),
        };

        assert_eq!(paths.page_dir(), PathBuf::from("/tmp/workspace/preprint"));
        assert_eq!(paths.meta_dir(), PathBuf::from("/tmp/workspace/.preprint"));
        assert_eq!(
            paths.artifacts_dir(),
            PathBuf::from("/tmp/workspace/.preprint/artifacts")
        );
        assert_eq!(
            paths.tab_artifacts_dir("gmail.com-t1"),
            PathBuf::from("/tmp/workspace/.preprint/artifacts/gmail.com-t1")
        );
        assert_eq!(
            paths.session_file("gmail.com-t1"),
            PathBuf::from("/tmp/workspace/.preprint/artifacts/gmail.com-t1/session.json")
        );
        assert_eq!(
            paths.screenshot_file("gmail.com-t1", "login-page"),
            PathBuf::from(
                "/tmp/workspace/.preprint/artifacts/gmail.com-t1/screenshots/login-page.png"
            )
        );
        assert_eq!(
            paths.recording_file("gmail.com-t1", "demo"),
            PathBuf::from(
                "/tmp/workspace/.preprint/artifacts/gmail.com-t1/recordings/demo.webm"
            )
        );
        assert_eq!(
            paths.logs_dir(),
            PathBuf::from("/tmp/workspace/.preprint/logs")
        );
        assert_eq!(
            paths.skill_md(),
            PathBuf::from("/tmp/workspace/.claude/skills/preprint-browser/SKILL.md")
        );
    }
}
