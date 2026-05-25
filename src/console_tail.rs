use std::fs::{self, OpenOptions};
use std::io::Write;

use time::OffsetDateTime;

use crate::agent_browser::{AgentBrowser, ErrorEntry};
use crate::paths::WorkspacePaths;
use crate::session::SessionState;

const MAX_LINES: usize = 500;

pub async fn poll_and_append(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    sessions: &[SessionState],
) {
    for state in sessions {
        if let Err(error) = poll_one(paths, agent_browser, state).await {
            tracing::warn!(
                tab_key = %state.tab_key,
                session = %state.session,
                tab_id = %state.tab_id,
                error = ?error,
                "console_tail poll failed"
            );
        }
    }
}

async fn poll_one(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    state: &SessionState,
) -> anyhow::Result<()> {
    let log_path = paths.tab_console_log(&state.tab_key);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !log_path.exists() {
        let header = format!(
            "# Live console + uncaught exceptions for {} — rolling {MAX_LINES}-line tail. Empty until the page logs.\n\n",
            state.tab_key
        );
        fs::write(&log_path, header)?;
    }

    let console = agent_browser
        .console_drain(&state.session, &state.tab_id)
        .await?;
    let errors = agent_browser
        .errors_snapshot(&state.session, &state.tab_id)
        .await?;
    let cursor = read_cursor(paths, &state.tab_key);
    let new_errors: &[ErrorEntry] = if cursor < errors.len() {
        &errors[cursor..]
    } else {
        &[]
    };

    if console.is_empty() && new_errors.is_empty() {
        return Ok(());
    }

    let stamp = now_utc_rfc3339();
    let mut buf = String::new();
    for entry in &console {
        let level = if entry.level.is_empty() {
            "log".to_string()
        } else {
            entry.level.clone()
        };
        buf.push_str(&format!("[{stamp}] {level}: {}\n", entry.text.trim_end()));
    }
    for entry in new_errors {
        buf.push_str(&format!(
            "[{stamp}] exception: {}\n",
            entry.text.trim_end()
        ));
        if let Some(url) = &entry.url {
            let line = entry.line.unwrap_or(0);
            let col = entry.column.unwrap_or(0);
            buf.push_str(&format!("  at {url}:{line}:{col}\n"));
        }
    }

    {
        let mut file = OpenOptions::new().append(true).open(&log_path)?;
        file.write_all(buf.as_bytes())?;
    }

    write_cursor(paths, &state.tab_key, errors.len())?;
    rotate_if_needed(paths, &state.tab_key)?;
    Ok(())
}

fn read_cursor(paths: &WorkspacePaths, tab_key: &str) -> usize {
    let path = paths.tab_errors_cursor(tab_key);
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    raw.trim().parse::<usize>().unwrap_or(0)
}

fn write_cursor(paths: &WorkspacePaths, tab_key: &str, value: usize) -> anyhow::Result<()> {
    let path = paths.tab_errors_cursor(tab_key);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, value.to_string())?;
    Ok(())
}

fn rotate_if_needed(paths: &WorkspacePaths, tab_key: &str) -> anyhow::Result<()> {
    let path = paths.tab_console_log(tab_key);
    let content = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    let lines: Vec<&str> = content.lines().collect();
    let header_lines = 2;
    let max_total = MAX_LINES + header_lines;
    if lines.len() <= max_total {
        return Ok(());
    }

    let header = lines.iter().take(header_lines).copied().collect::<Vec<_>>();
    let tail_start = lines.len().saturating_sub(MAX_LINES);
    let tail = lines.iter().skip(tail_start).copied().collect::<Vec<_>>();

    let mut rewritten = header.join("\n");
    rewritten.push('\n');
    rewritten.push_str(&tail.join("\n"));
    if !rewritten.ends_with('\n') {
        rewritten.push('\n');
    }
    fs::write(&path, rewritten)?;
    Ok(())
}

fn now_utc_rfc3339() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
    )
}
