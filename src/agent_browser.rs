use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;
use tokio::process::Command;

use crate::action::Action;

#[derive(Debug, Clone)]
pub struct AgentBrowser {
    workspace: PathBuf,
    binary: PathBuf,
}

#[derive(Debug, Clone)]
pub struct BrowserSnapshot {
    pub origin: Option<String>,
    pub snapshot: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct TabInfo {
    #[serde(rename = "tabId")]
    pub tab_id: String,
    pub label: Option<String>,
    #[serde(default)]
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub active: bool,
    #[serde(default, rename = "type")]
    pub target_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentBrowserResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SnapshotData {
    origin: Option<String>,
    snapshot: String,
}

#[derive(Debug, Deserialize)]
struct TabListData {
    tabs: Vec<TabInfo>,
}

#[derive(Debug, Deserialize)]
struct NewTabData {
    #[serde(rename = "tabId")]
    tab_id: String,
    label: Option<String>,
    url: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ScreenshotData {
    path: String,
    #[serde(default)]
    annotations: Vec<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct RecordStartData {
    path: String,
    #[serde(default)]
    started: bool,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct RecordStopData {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConsoleEntry {
    #[serde(default, rename = "type")]
    pub level: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEntry {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
    #[serde(default)]
    pub column: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ConsoleData {
    #[serde(default)]
    messages: Vec<ConsoleEntry>,
}

#[derive(Debug, Deserialize)]
struct ErrorsData {
    #[serde(default)]
    errors: Vec<ErrorEntry>,
}

impl AgentBrowser {
    pub fn new(workspace: PathBuf, binary: PathBuf) -> Self {
        Self { workspace, binary }
    }

    #[tracing::instrument(skip(self), fields(session = %session, url = %url, headed, profile))]
    pub async fn open(
        &self,
        session: &str,
        url: &str,
        headed: bool,
        profile: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut prefix: Vec<String> = Vec::new();
        if let Some(p) = profile {
            prefix.push("--profile".to_string());
            prefix.push(p.to_string());
        }
        let mut args: Vec<String> = prefix;
        args.push("open".to_string());
        args.push(url.to_string());
        args.push("--json".to_string());
        if headed {
            args.push("--headed".to_string());
        }
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(session, &borrowed).await.map(|_| ())
    }

    #[tracing::instrument(skip(self), fields(session = %session))]
    pub async fn close(&self, session: &str) -> anyhow::Result<()> {
        self.run(session, &["close", "--json"]).await.map(|_| ())
    }

    #[tracing::instrument(skip(self), fields(session = %session))]
    pub async fn tab_list(&self, session: &str) -> anyhow::Result<Vec<TabInfo>> {
        let output = self
            .run(session, &["tab", "list", "--json"])
            .await
            .context("failed to list agent-browser tabs")?;
        let data: TabListData = parse_agent_browser_response(&output, "tab list")?;
        Ok(data.tabs)
    }

    #[tracing::instrument(skip(self), fields(session = %session, tab_id = %tab_id, activate))]
    pub async fn switch_tab(
        &self,
        session: &str,
        tab_id: &str,
        activate: bool,
    ) -> anyhow::Result<()> {
        let args: &[&str] = if activate {
            &["tab", tab_id, "--json"]
        } else {
            &["tab", tab_id, "--no-activate", "--json"]
        };
        let output = self
            .run(session, args)
            .await
            .with_context(|| format!("failed to switch agent-browser to tab {tab_id}"))?;
        let _: serde_json::Value = parse_agent_browser_response(&output, &format!("tab {tab_id}"))?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(session = %session, url = %url))]
    pub async fn new_tab(&self, session: &str, url: &str) -> anyhow::Result<TabInfo> {
        let output = self
            .run(session, &["tab", "new", url, "--json"])
            .await
            .with_context(|| format!("failed to create new tab for {url}"))?;
        let data: NewTabData = parse_agent_browser_response(&output, "tab new")?;

        let tabs = self.tab_list(session).await.unwrap_or_default();
        if let Some(tab) = tabs.into_iter().find(|tab| tab.tab_id == data.tab_id) {
            return Ok(tab);
        }

        Ok(TabInfo {
            tab_id: data.tab_id,
            label: data.label,
            title: String::new(),
            url: data.url,
            active: true,
            target_type: Some("page".to_string()),
        })
    }

    #[tracing::instrument(skip(self), fields(session = %session, tab_id = %tab_id))]
    pub async fn close_tab(&self, session: &str, tab_id: &str) -> anyhow::Result<()> {
        let output = self
            .run(session, &["tab", "close", tab_id, "--json"])
            .await
            .with_context(|| format!("failed to close agent-browser tab {tab_id}"))?;
        let _: serde_json::Value =
            parse_agent_browser_response(&output, &format!("tab close {tab_id}"))?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(session = %session))]
    pub async fn snapshot(&self, session: &str) -> anyhow::Result<BrowserSnapshot> {
        let output = self
            .run(session, &["snapshot", "-i", "--json"])
            .await
            .context("failed to get agent-browser snapshot")?;

        let data: SnapshotData = parse_agent_browser_response(&output, "snapshot")?;

        Ok(BrowserSnapshot {
            origin: data.origin,
            snapshot: data.snapshot,
        })
    }

    #[tracing::instrument(skip(self), fields(session = %session, action = ?action))]
    pub async fn execute(&self, session: &str, action: &Action) -> anyhow::Result<()> {
        match action {
            Action::Goto(url) => self.open(session, url, false, None).await,
            Action::Snapshot => Ok(()),
            Action::Click(reference) => {
                self.run_json_action(session, &["click", &format_ref(reference)])
                    .await
            }
            Action::Fill { reference, text } => {
                self.run_json_action(session, &["fill", &format_ref(reference), text])
                    .await
            }
            Action::Type { reference, text } => {
                self.run_json_action(session, &["type", &format_ref(reference), text])
                    .await
            }
            Action::Press(key) => self.run_json_action(session, &["press", key]).await,
            Action::WaitText(text) => {
                self.run_json_action(session, &["wait", "--text", text])
                    .await
            }
            Action::WaitUrl(pattern) => {
                self.run_json_action(session, &["wait", "--url", pattern])
                    .await
            }
            Action::WaitIdle => {
                self.run_json_action(session, &["wait", "--load", "networkidle"])
                    .await
            }
            Action::Scroll { direction, amount } => {
                let amount = amount.to_string();
                self.run_json_action(session, &["scroll", direction, &amount])
                    .await
            }
            Action::Back => self.run_json_action(session, &["back"]).await,
            Action::Reload => self.run_json_action(session, &["reload"]).await,
            Action::Screenshot { .. } | Action::RecordStart { .. } | Action::RecordStop => {
                anyhow::bail!(
                    "{action} is a side-effect action; route via daemon::dispatch_action"
                )
            }
        }
    }

    #[tracing::instrument(skip(self, path), fields(session = %session, annotate))]
    pub async fn screenshot(
        &self,
        session: &str,
        path: &Path,
        annotate: bool,
    ) -> anyhow::Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let mut args: Vec<&str> = vec!["screenshot", &path_str];
        if annotate {
            args.push("--annotate");
        }
        args.push("--json");
        let output = self
            .run(session, &args)
            .await
            .with_context(|| format!("failed to take screenshot to {}", path.display()))?;
        let _: ScreenshotData = parse_agent_browser_response(&output, "screenshot")?;
        Ok(())
    }

    #[tracing::instrument(skip(self, path), fields(session = %session))]
    pub async fn record_start(&self, session: &str, path: &Path) -> anyhow::Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let args: [&str; 5] = ["record", "start", &path_str, "--in-place", "--json"];
        let output = self
            .run(session, &args)
            .await
            .with_context(|| format!("failed to start recording to {}", path.display()))?;
        let _: RecordStartData = parse_agent_browser_response(&output, "record start")?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(session = %session))]
    pub async fn record_stop(&self, session: &str) -> anyhow::Result<()> {
        let args: [&str; 3] = ["record", "stop", "--json"];
        let output = self
            .run(session, &args)
            .await
            .context("failed to stop recording")?;
        let _: RecordStopData = parse_agent_browser_response(&output, "record stop")?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(session = %session, tab = %tab_id))]
    pub async fn console_drain(
        &self,
        session: &str,
        tab_id: &str,
    ) -> anyhow::Result<Vec<ConsoleEntry>> {
        let args: [&str; 5] = ["console", "--clear", "--tab", tab_id, "--json"];
        let output = self
            .run(session, &args)
            .await
            .context("failed to drain console")?;
        let data: ConsoleData = parse_agent_browser_response(&output, "console")?;
        Ok(data.messages)
    }

    #[tracing::instrument(skip(self), fields(session = %session, tab = %tab_id))]
    pub async fn errors_snapshot(
        &self,
        session: &str,
        tab_id: &str,
    ) -> anyhow::Result<Vec<ErrorEntry>> {
        let args: [&str; 4] = ["errors", "--tab", tab_id, "--json"];
        let output = self
            .run(session, &args)
            .await
            .context("failed to read errors")?;
        let data: ErrorsData = parse_agent_browser_response(&output, "errors")?;
        Ok(data.errors)
    }

    async fn run_json_action(&self, session: &str, args: &[&str]) -> anyhow::Result<()> {
        let mut owned = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        owned.push("--json".to_string());
        let borrowed = owned.iter().map(String::as_str).collect::<Vec<_>>();
        let output = self.run(session, &borrowed).await?;
        let _: serde_json::Value = parse_agent_browser_response(&output, "action")?;
        Ok(())
    }

    #[tracing::instrument(skip(self, args), fields(session = %session, args = ?args))]
    async fn run(&self, session: &str, args: &[&str]) -> anyhow::Result<String> {
        let mut command = self.base_command();
        command.current_dir(&self.workspace);
        command.args(args);
        command.arg("--session").arg(session);

        let output = command
            .output()
            .await
            .with_context(|| format!("failed to run agent-browser command: {}", args.join(" ")))?;

        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                exit = ?output.status.code(),
                stdout = %stdout,
                stderr = %stderr,
                "agent-browser command failed"
            );
            anyhow::bail!(
                "agent-browser command failed: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
                args.join(" "),
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn base_command(&self) -> Command {
        Command::new(&self.binary)
    }
}

fn format_ref(reference: &str) -> String {
    if reference.starts_with('@') {
        reference.to_string()
    } else {
        format!("@{reference}")
    }
}

fn parse_agent_browser_response<T>(output: &str, command: &str) -> anyhow::Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let response: AgentBrowserResponse<T> = serde_json::from_str(output).with_context(|| {
        format!(
            "agent-browser {command} did not return expected json: {}",
            output.trim()
        )
    })?;

    if !response.success {
        anyhow::bail!(
            "agent-browser {command} failed: {}",
            response
                .error
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }

    response
        .data
        .ok_or_else(|| anyhow::anyhow!("agent-browser {command} response had no data"))
}
