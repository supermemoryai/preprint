use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::paths::WorkspacePaths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordingState {
    pub name: String,
    pub path: PathBuf,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub tab_key: String,
    pub tab_id: String,
    pub initial_host: String,
    pub session: String,
    pub initial_url: String,
    pub current_url: String,
    pub context: String,
    pub opened_at: String,
    pub last_action: String,
    pub last_action_hash: Option<String>,
    pub last_snapshot_hash: Option<String>,
    #[serde(default)]
    pub recording: Option<RecordingState>,
    #[serde(default)]
    pub profile_name: Option<String>,
}

impl SessionState {
    pub fn new(
        tab_key: String,
        tab_id: String,
        initial_host: String,
        session: String,
        initial_url: String,
        current_url: String,
        context: String,
        opened_at: String,
        profile_name: Option<String>,
    ) -> Self {
        Self {
            tab_key,
            tab_id,
            initial_host,
            session,
            initial_url,
            current_url,
            context,
            opened_at,
            last_action: "none".to_string(),
            last_action_hash: None,
            last_snapshot_hash: None,
            recording: None,
            profile_name,
        }
    }

    pub fn save(&self, paths: &WorkspacePaths) -> anyhow::Result<()> {
        paths.ensure()?;
        let dest = paths.session_file(&self.tab_key);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create tab artifacts dir {}", parent.display())
            })?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&dest, json).with_context(|| {
            format!("failed to write session metadata {}", dest.display())
        })
    }

    pub fn load(paths: &WorkspacePaths, tab_key: &str) -> anyhow::Result<Self> {
        let path = paths.session_file(tab_key);
        let json = fs::read_to_string(&path)
            .with_context(|| format!("failed to read session file {}", path.display()))?;
        serde_json::from_str(&json)
            .with_context(|| format!("failed to parse session file {}", path.display()))
    }

    pub fn list(paths: &WorkspacePaths) -> anyhow::Result<Vec<Self>> {
        let mut sessions: Vec<Self> = Vec::new();

        if !paths.artifacts_dir().exists() {
            return Ok(sessions);
        }

        for entry in fs::read_dir(paths.artifacts_dir()).with_context(|| {
            format!(
                "failed to read artifacts directory {}",
                paths.artifacts_dir().display()
            )
        })? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(tab_key) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let session_file = paths.session_file(&tab_key);
            if !session_file.exists() {
                continue;
            }
            let json = fs::read_to_string(&session_file).with_context(|| {
                format!("failed to read session file {}", session_file.display())
            })?;
            sessions.push(serde_json::from_str(&json).with_context(|| {
                format!("failed to parse session file {}", session_file.display())
            })?);
        }

        sessions.sort_by(|a, b| {
            a.opened_at
                .cmp(&b.opened_at)
                .then_with(|| a.tab_key.cmp(&b.tab_key))
        });
        Ok(sessions)
    }
}
