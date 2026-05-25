use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions as FsOpenOptions};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Context;
use time::OffsetDateTime;
use tokio::time::sleep;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::action::{Action, first_pending_action};
use crate::agent_browser::{AgentBrowser, BrowserSnapshot, TabInfo};
use crate::diff::render_diff;
use crate::document::{render_error_page, render_page, render_tabs_manifest};
use crate::inject::write_skill_file;
use crate::paths::{ACTION_MARKER, WorkspacePaths, short_hash, site_from_url, tab_key};
use crate::session::{RecordingState, SessionState};

#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub url: String,
    pub session: String,
    pub context: Option<String>,
    pub preview: bool,
    pub inject: bool,
    pub spawn_daemon: bool,
    pub poll_ms: u64,
    pub dev: bool,
    pub profile: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DaemonOptions {
    pub poll_ms: u64,
    pub once: bool,
    pub inject: bool,
}

#[tracing::instrument(skip_all, fields(url = %options.url, session = %options.session, preview = options.preview))]
pub async fn open(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    options: OpenOptions,
) -> anyhow::Result<()> {
    paths.ensure()?;
    if options.inject {
        write_skill_file(paths)?;
    }

    let initial_host = site_from_url(&options.url)?;
    let session_name = options.session.clone();

    let existing_profile = SessionState::list(paths)?
        .into_iter()
        .find(|s| s.session == session_name)
        .map(|s| s.profile_name);

    let (effective_profile, launching_session) = match existing_profile {
        Some(existing) => {
            if let Some(message) = profile_mismatch_warning(
                &session_name,
                existing.as_deref(),
                options.profile.as_deref(),
            ) {
                eprintln!("{message}");
            }
            (existing, false)
        }
        None => (options.profile.clone(), true),
    };

    let tab = if launching_session {
        agent_browser
            .open(
                &session_name,
                &options.url,
                options.preview,
                effective_profile.as_deref(),
            )
            .await
            .with_context(|| format!("failed to open {}", options.url))?;
        let tabs = agent_browser.tab_list(&session_name).await?;
        find_opened_tab(tabs, &options.url)?
    } else {
        agent_browser
            .new_tab(&session_name, &options.url)
            .await
            .with_context(|| format!("failed to open new tab for {}", options.url))?
    };

    let tab_key = tab_key(&initial_host, &tab.tab_id, &session_name);
    agent_browser
        .switch_tab(&session_name, &tab.tab_id, true)
        .await?;
    let snapshot = agent_browser.snapshot(&session_name).await?;
    let current_url = snapshot.origin.clone().unwrap_or_else(|| tab.url.clone());

    let mut session = SessionState::new(
        tab_key.clone(),
        tab.tab_id,
        initial_host,
        session_name,
        options.url,
        current_url,
        options.context.unwrap_or_default(),
        now_utc_rfc3339(),
        effective_profile,
    );
    write_snapshot(paths, &mut session, &snapshot).await?;
    write_tabs_manifest_file(paths)?;

    if options.spawn_daemon {
        spawn_daemon(paths, options.poll_ms, options.inject, options.dev)?;
    }

    println!("{}", paths.page_file(&tab_key).display());
    println!("{}", paths.diff_file(&tab_key).display());
    Ok(())
}

fn profile_mismatch_warning(
    session_name: &str,
    existing: Option<&str>,
    requested: Option<&str>,
) -> Option<String> {
    match (existing, requested) {
        (Some(have), Some(want)) if have == want => None,
        (None, None) => None,
        (Some(have), Some(want)) => Some(format!(
            "warning: session '{session_name}' is locked to profile '{have}'; --profile '{want}' ignored"
        )),
        (Some(have), None) => Some(format!(
            "warning: session '{session_name}' is locked to profile '{have}'; --no-profile ignored"
        )),
        (None, Some(want)) => Some(format!(
            "warning: session '{session_name}' is running clean (no profile); --profile '{want}' ignored"
        )),
    }
}

#[tracing::instrument(skip_all, fields(poll_ms = options.poll_ms, once = options.once))]
pub async fn run(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    options: DaemonOptions,
) -> anyhow::Result<()> {
    paths.ensure()?;
    if options.inject {
        write_skill_file(paths)?;
    }
    fs::write(paths.daemon_pid(), std::process::id().to_string()).with_context(|| {
        format!(
            "failed to write daemon pid {}",
            paths.daemon_pid().display()
        )
    })?;
    tracing::info!(
        workspace = %paths.root().display(),
        poll_ms = options.poll_ms,
        "daemon started"
    );

    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("failed to install SIGTERM handler")?;
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .context("failed to install SIGINT handler")?;

    loop {
        if let Err(error) = process_all_sessions(paths, agent_browser).await {
            if options.once {
                return Err(error);
            }

            tracing::error!(error = ?error, "daemon sync error");
        }

        if options.once {
            break;
        }

        if SessionState::list(paths)
            .map(|s| s.is_empty())
            .unwrap_or(false)
        {
            tracing::info!("no tabs remaining; daemon exiting");
            shutdown_runtime(paths, agent_browser).await?;
            break;
        }

        #[cfg(unix)]
        tokio::select! {
            _ = sleep(Duration::from_millis(options.poll_ms)) => {}
            _ = terminate.recv() => {
                shutdown_runtime(paths, agent_browser).await?;
                break;
            }
            _ = interrupt.recv() => {
                shutdown_runtime(paths, agent_browser).await?;
                break;
            }
        }

        #[cfg(not(unix))]
        tokio::select! {
            _ = sleep(Duration::from_millis(options.poll_ms)) => {}
            _ = tokio::signal::ctrl_c() => {
                shutdown_runtime(paths, agent_browser).await?;
                break;
            }
        }
    }

    Ok(())
}

#[tracing::instrument(skip_all, fields(tab_key = ?tab_key))]
pub async fn sync(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    tab_key: Option<String>,
) -> anyhow::Result<()> {
    paths.ensure()?;
    if let Some(tab_key) = tab_key {
        let mut session = SessionState::load(paths, &tab_key)?;
        process_session(paths, agent_browser, &mut session).await?;
        write_tabs_manifest_file(paths)?;
    } else {
        process_all_sessions(paths, agent_browser).await?;
    }
    Ok(())
}

pub fn status(paths: &WorkspacePaths) -> anyhow::Result<()> {
    let sessions = SessionState::list(paths)?;

    println!("workspace: {}", paths.root().display());
    println!("files_dir: {}", paths.page_dir().display());
    println!("meta_dir: {}", paths.meta_dir().display());
    println!("daemon: {}", daemon_status(paths));
    println!("tabs: {}", sessions.len());

    for session in sessions {
        println!(
            "- {} tab_id={} url={} last_action={}",
            session.tab_key, session.tab_id, session.current_url, session.last_action
        );
    }

    Ok(())
}

pub async fn close_tab(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    tab_key: &str,
) -> anyhow::Result<()> {
    paths.ensure()?;
    let session = SessionState::load(paths, tab_key)?;

    if session.recording.is_some() {
        if let Err(error) = agent_browser.record_stop(&session.session).await {
            tracing::warn!(
                tab_key = %tab_key,
                error = ?error,
                "failed to stop recording before closing tab"
            );
        }
    }

    match agent_browser
        .close_tab(&session.session, &session.tab_id)
        .await
    {
        Ok(()) => cleanup_tab_files(paths, &session)?,
        Err(error) if is_last_tab_close_error(&error) => {
            agent_browser.close(&session.session).await?;
            cleanup_agent_session_files(paths, &session.session)?;
        }
        Err(error) if is_missing_tab_error(&error) => {
            cleanup_tab_files(paths, &session)?;
        }
        Err(error) => return Err(error),
    }

    if SessionState::list(paths)?.is_empty() {
        sweep_workspace_files(paths)?;
    } else {
        write_tabs_manifest_file(paths)?;
    }
    println!("closed {tab_key}");
    Ok(())
}

pub async fn stop(paths: &WorkspacePaths, agent_browser: &AgentBrowser) -> anyhow::Result<()> {
    let sessions = SessionState::list(paths).unwrap_or_default();

    let Ok(pid) = fs::read_to_string(paths.daemon_pid()) else {
        close_browser_sessions(agent_browser, &sessions).await;
        for session in &sessions {
            let _ = cleanup_tab_files(paths, session);
        }
        cleanup_runtime(paths)?;
        let _ = sweep_workspace_files(paths);
        println!("daemon: stopped");
        return Ok(());
    };
    let Ok(pid) = pid.trim().parse::<u32>() else {
        close_browser_sessions(agent_browser, &sessions).await;
        for session in &sessions {
            let _ = cleanup_tab_files(paths, session);
        }
        cleanup_runtime(paths)?;
        let _ = sweep_workspace_files(paths);
        println!("daemon: stopped");
        return Ok(());
    };

    if !daemon_is_running(paths) {
        close_browser_sessions(agent_browser, &sessions).await;
        for session in &sessions {
            let _ = cleanup_tab_files(paths, session);
        }
        cleanup_runtime(paths)?;
        let _ = sweep_workspace_files(paths);
        println!("daemon: stopped");
        return Ok(());
    }

    let status = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .context("failed to stop preprint daemon")?;

    if status.success() {
        close_browser_sessions(agent_browser, &sessions).await;
        for session in &sessions {
            let _ = cleanup_tab_files(paths, session);
        }
        cleanup_runtime(paths)?;
        let _ = sweep_workspace_files(paths);
        println!("daemon: stopped");
        Ok(())
    } else {
        anyhow::bail!("failed to stop daemon pid {pid}")
    }
}

async fn process_all_sessions(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
) -> anyhow::Result<()> {
    let sessions = SessionState::list(paths)?;
    if sessions.is_empty() {
        sweep_workspace_files(paths)?;
        return Ok(());
    }

    let agent_sessions = sessions
        .iter()
        .map(|session| session.session.clone())
        .collect::<BTreeSet<_>>();

    for agent_session in &agent_sessions {
        let agent_session = agent_session.clone();
        let tracked = SessionState::list(paths)?
            .into_iter()
            .filter(|session| session.session == agent_session)
            .collect::<Vec<_>>();
        if tracked.is_empty() {
            continue;
        }

        let tabs = agent_browser.tab_list(&agent_session).await?;
        reconcile_agent_session(paths, agent_browser, &agent_session, &tracked, &tabs).await?;

        let current = SessionState::list(paths)?
            .into_iter()
            .filter(|session| session.session == agent_session)
            .collect::<Vec<_>>();

        for mut session in current {
            if !has_trackable_tab(&tabs, &session.tab_id) {
                continue;
            }

            if let Err(error) = process_session(paths, agent_browser, &mut session).await {
                if is_missing_tab_error(&error) {
                    cleanup_tab_files(paths, &session)?;
                } else {
                    return Err(error);
                }
            }
        }

        let remaining = SessionState::list(paths)?
            .iter()
            .filter(|session| session.session == agent_session)
            .count();
        if remaining == 0 && !tabs.iter().any(is_trackable_tab) {
            if let Err(error) = agent_browser.close(&agent_session).await {
                tracing::debug!(
                    session = %agent_session,
                    error = ?error,
                    "failed to close empty agent-browser session"
                );
            }
        }
    }

    let open_sessions = SessionState::list(paths)?;
    crate::console_tail::poll_and_append(paths, agent_browser, &open_sessions).await;

    if SessionState::list(paths)?.is_empty() {
        sweep_workspace_files(paths)
    } else {
        write_tabs_manifest_file(paths)
    }
}

async fn reconcile_agent_session(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    agent_session: &str,
    tracked: &[SessionState],
    tabs: &[TabInfo],
) -> anyhow::Result<()> {
    let trackable_tabs = tabs
        .iter()
        .filter(|tab| is_trackable_tab(tab))
        .collect::<Vec<_>>();
    let trackable_ids = trackable_tabs
        .iter()
        .map(|tab| tab.tab_id.as_str())
        .collect::<BTreeSet<_>>();

    for session in tracked {
        if !trackable_ids.contains(session.tab_id.as_str()) {
            cleanup_tab_files(paths, session)?;
        }
    }

    let mut known_by_tab_id = SessionState::list(paths)?
        .into_iter()
        .filter(|session| session.session == agent_session)
        .map(|session| (session.tab_id.clone(), session))
        .collect::<BTreeMap<_, _>>();

    for tab in trackable_tabs {
        if let Some(session) = known_by_tab_id.get_mut(&tab.tab_id) {
            if session.current_url != tab.url {
                session.current_url = tab.url.clone();
                session.save(paths)?;
            }
            continue;
        }

        create_observed_session(paths, agent_browser, agent_session, tab).await?;
    }

    Ok(())
}

async fn create_observed_session(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    agent_session: &str,
    tab: &TabInfo,
) -> anyhow::Result<()> {
    let Some(initial_host) = initial_host_from_tab(tab) else {
        return Ok(());
    };
    let tab_key = tab_key(&initial_host, &tab.tab_id, agent_session);
    let inherited_profile = SessionState::list(paths)?
        .into_iter()
        .find(|s| s.session == agent_session)
        .and_then(|s| s.profile_name);
    let mut session = SessionState::new(
        tab_key,
        tab.tab_id.clone(),
        initial_host,
        agent_session.to_string(),
        tab.url.clone(),
        tab.url.clone(),
        String::new(),
        now_utc_rfc3339(),
        inherited_profile,
    );

    agent_browser
        .switch_tab(agent_session, &tab.tab_id, false)
        .await?;
    let snapshot = agent_browser.snapshot(agent_session).await?;
    write_snapshot(paths, &mut session, &snapshot).await
}

#[tracing::instrument(skip_all, fields(tab_key = %session.tab_key, tab_id = %session.tab_id, session = %session.session))]
async fn process_session(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    session: &mut SessionState,
) -> anyhow::Result<()> {
    agent_browser
        .switch_tab(&session.session, &session.tab_id, false)
        .await
        .with_context(|| format!("failed to select tab {}", session.tab_id))?;

    let page_path = paths.page_file(&session.tab_key);
    let markdown = match fs::read_to_string(&page_path) {
        Ok(markdown) => markdown,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let snapshot = agent_browser.snapshot(&session.session).await?;
            write_snapshot(paths, session, &snapshot).await?;
            return Ok(());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read page file {}", page_path.display()));
        }
    };

    let Some(raw_action) = first_pending_action(&markdown, ACTION_MARKER)? else {
        return observe_browser_change(paths, agent_browser, session, &markdown).await;
    };

    let action_hash = short_hash(&raw_action);
    if session.last_action_hash.as_deref() == Some(action_hash.as_str()) {
        return Ok(());
    }

    let action = match Action::parse(&raw_action) {
        Ok(action) => action,
        Err(error) => {
            session.last_action = format!("error parsing `{raw_action}`: {error}");
            let rendered = render_error_page(session, &markdown, &session.last_action);
            fs::write(&page_path, rendered)
                .with_context(|| format!("failed to write page file {}", page_path.display()))?;
            session.last_action_hash = Some(action_hash);
            session.save(paths)?;
            return Ok(());
        }
    };

    agent_browser
        .switch_tab(&session.session, &session.tab_id, true)
        .await
        .with_context(|| format!("failed to activate tab {}", session.tab_id))?;
    agent_browser.snapshot(&session.session).await?;

    let before = markdown;
    let last_action = match dispatch_action(paths, agent_browser, session, &action).await {
        Ok(message) => message,
        Err(error) => {
            session.last_action = format!("error {action}: {error}");
            let rendered = render_error_page(session, &before, &session.last_action);
            fs::write(&page_path, rendered).with_context(|| {
                format!("failed to write page file {}", page_path.display())
            })?;
            session.last_action_hash = Some(action_hash);
            session.save(paths)?;
            return Ok(());
        }
    };

    let snapshot = agent_browser.snapshot(&session.session).await?;
    session.last_action = last_action;
    session.last_action_hash = Some(action_hash);
    write_snapshot_with_previous(paths, session, &snapshot, &before).await
}

async fn dispatch_action(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    session: &mut SessionState,
    action: &Action,
) -> anyhow::Result<String> {
    match action {
        Action::Screenshot { name, annotate } => {
            let resolved = name.clone().unwrap_or_else(timestamp_capture_name);
            let dest = paths.screenshot_file(&session.tab_key, &resolved);
            ensure_parent_dir(&dest)?;
            agent_browser
                .screenshot(&session.session, &dest, *annotate)
                .await?;
            Ok(format!("ok screenshot -> {}", dest.display()))
        }
        Action::RecordStart { name } => {
            if let Some(active) = &session.recording {
                anyhow::bail!("recording already active: {}", active.name);
            }
            let resolved = name.clone().unwrap_or_else(timestamp_capture_name);
            let dest = paths.recording_file(&session.tab_key, &resolved);
            ensure_parent_dir(&dest)?;
            agent_browser.record_start(&session.session, &dest).await?;
            session.recording = Some(RecordingState {
                name: resolved,
                path: dest.clone(),
                started_at: now_utc_rfc3339(),
            });
            Ok(format!("ok record_start -> {}", dest.display()))
        }
        Action::RecordStop => {
            let recording = session
                .recording
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no active recording to stop"))?;
            agent_browser.record_stop(&session.session).await?;
            session.recording = None;
            Ok(format!("ok record_stop -> {}", recording.path.display()))
        }
        _ => {
            agent_browser.execute(&session.session, action).await?;
            Ok(format!("ok {action}"))
        }
    }
}

fn ensure_parent_dir(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }
    Ok(())
}

fn timestamp_capture_name() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[tracing::instrument(skip_all, fields(tab_key = %session.tab_key, tab_id = %session.tab_id, session = %session.session))]
async fn observe_browser_change(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
    session: &mut SessionState,
    current_markdown: &str,
) -> anyhow::Result<()> {
    let snapshot = agent_browser.snapshot(&session.session).await?;
    let mut probe = session.clone();
    if let Some(origin) = &snapshot.origin {
        probe.current_url = origin.clone();
    }

    let rendered = render_page(&probe, &snapshot);
    let rendered_hash = short_hash(&rendered);
    if session.last_snapshot_hash.as_deref() == Some(rendered_hash.as_str())
        || rendered == current_markdown
    {
        if session.current_url != probe.current_url {
            session.current_url = probe.current_url;
            session.save(paths)?;
        }
        return Ok(());
    }

    session.last_action_hash = None;
    write_snapshot_with_previous(paths, session, &snapshot, current_markdown).await
}

async fn write_snapshot(
    paths: &WorkspacePaths,
    session: &mut SessionState,
    snapshot: &BrowserSnapshot,
) -> anyhow::Result<()> {
    let previous = fs::read_to_string(paths.page_file(&session.tab_key)).unwrap_or_default();
    write_snapshot_with_previous(paths, session, snapshot, &previous).await
}

async fn write_snapshot_with_previous(
    paths: &WorkspacePaths,
    session: &mut SessionState,
    snapshot: &BrowserSnapshot,
    previous: &str,
) -> anyhow::Result<()> {
    paths.ensure()?;

    if let Some(origin) = &snapshot.origin {
        session.current_url = origin.clone();
    }

    let rendered = render_page(session, snapshot);
    let diff = render_diff(&session.tab_key, &session.last_action, previous, &rendered);

    fs::write(paths.page_file(&session.tab_key), &rendered).with_context(|| {
        format!(
            "failed to write page file {}",
            paths.page_file(&session.tab_key).display()
        )
    })?;
    fs::write(paths.diff_file(&session.tab_key), diff).with_context(|| {
        format!(
            "failed to write diff file {}",
            paths.diff_file(&session.tab_key).display()
        )
    })?;

    session.last_snapshot_hash = Some(short_hash(&rendered));
    session.save(paths)
}

fn spawn_daemon(
    paths: &WorkspacePaths,
    poll_ms: u64,
    inject: bool,
    dev: bool,
) -> anyhow::Result<()> {
    if daemon_is_running(paths) {
        return Ok(());
    }

    let exe = std::env::current_exe().context("failed to locate current preprint executable")?;

    let mut command = Command::new(exe);
    command.arg("--workspace").arg(paths.root());
    if dev {
        command.arg("--dev");
    }
    command
        .arg("daemon")
        .arg("--poll-ms")
        .arg(poll_ms.to_string());

    if inject {
        command.arg("--inject");
    }

    command.stdin(Stdio::null());
    if dev {
        let log = FsOpenOptions::new()
            .create(true)
            .append(true)
            .open(paths.daemon_log())
            .with_context(|| {
                format!("failed to open daemon log {}", paths.daemon_log().display())
            })?;
        let log_err = log
            .try_clone()
            .context("failed to clone daemon log handle")?;
        command.stdout(Stdio::from(log)).stderr(Stdio::from(log_err));
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }

    detach_command(&mut command);

    command.spawn().context("failed to spawn preprint daemon")?;

    Ok(())
}

#[cfg(unix)]
fn detach_command(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_command(_command: &mut Command) {}

fn cleanup_runtime(paths: &WorkspacePaths) -> anyhow::Result<()> {
    remove_dir_if_exists(paths.meta_dir())
}

async fn shutdown_runtime(
    paths: &WorkspacePaths,
    agent_browser: &AgentBrowser,
) -> anyhow::Result<()> {
    let sessions = SessionState::list(paths).unwrap_or_default();
    close_browser_sessions(agent_browser, &sessions).await;
    cleanup_runtime(paths)
}

async fn close_browser_sessions(agent_browser: &AgentBrowser, sessions: &[SessionState]) {
    let mut seen = BTreeSet::new();
    for session in sessions {
        if !seen.insert(session.session.as_str()) {
            continue;
        }

        if let Err(error) = agent_browser.close(&session.session).await {
            tracing::warn!(
                session = %session.session,
                error = ?error,
                "failed to close browser session on stop"
            );
        }
    }
}

fn cleanup_agent_session_files(paths: &WorkspacePaths, agent_session: &str) -> anyhow::Result<()> {
    for session in SessionState::list(paths)?
        .into_iter()
        .filter(|session| session.session == agent_session)
    {
        cleanup_tab_files(paths, &session)?;
    }
    Ok(())
}

fn cleanup_tab_files(paths: &WorkspacePaths, session: &SessionState) -> anyhow::Result<()> {
    remove_file_if_exists(paths.page_file(&session.tab_key))?;
    remove_file_if_exists(paths.diff_file(&session.tab_key))?;
    remove_dir_if_exists(paths.tab_artifacts_dir(&session.tab_key))
}

fn sweep_workspace_files(paths: &WorkspacePaths) -> anyhow::Result<()> {
    remove_file_if_exists(paths.tabs_manifest())?;
    let _ = fs::remove_dir_all(paths.page_dir());
    let _ = fs::remove_dir_all(paths.artifacts_dir());

    remove_file_if_exists(paths.skill_md())?;
    let skill_md = paths.skill_md();
    let mut ancestor = skill_md.parent();
    while let Some(dir) = ancestor {
        if dir == paths.root() || fs::remove_dir(dir).is_err() {
            break;
        }
        ancestor = dir.parent();
    }

    Ok(())
}

fn remove_file_if_exists(path: std::path::PathBuf) -> anyhow::Result<()> {
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn remove_dir_if_exists(path: std::path::PathBuf) -> anyhow::Result<()> {
    match fs::remove_dir_all(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn write_tabs_manifest_file(paths: &WorkspacePaths) -> anyhow::Result<()> {
    paths.ensure()?;
    let sessions = SessionState::list(paths)?;
    fs::write(paths.tabs_manifest(), render_tabs_manifest(&sessions)).with_context(|| {
        format!(
            "failed to write tabs manifest {}",
            paths.tabs_manifest().display()
        )
    })
}

fn find_opened_tab(tabs: Vec<TabInfo>, requested_url: &str) -> anyhow::Result<TabInfo> {
    let requested_host = site_from_url(requested_url).ok();

    if let Some(tab) = tabs
        .iter()
        .find(|tab| tab.active && is_trackable_tab(tab))
        .cloned()
    {
        return Ok(tab);
    }

    if let Some(host) = requested_host {
        if let Some(tab) = tabs
            .iter()
            .find(|tab| initial_host_from_tab(tab).as_deref() == Some(host.as_str()))
            .cloned()
        {
            return Ok(tab);
        }
    }

    tabs.into_iter()
        .find(is_trackable_tab)
        .ok_or_else(|| anyhow::anyhow!("agent-browser did not report an opened tab"))
}

fn has_trackable_tab(tabs: &[TabInfo], tab_id: &str) -> bool {
    tabs.iter()
        .any(|tab| tab.tab_id == tab_id && is_trackable_tab(tab))
}

fn is_trackable_tab(tab: &TabInfo) -> bool {
    if tab
        .target_type
        .as_deref()
        .is_some_and(|kind| kind != "page")
    {
        return false;
    }

    initial_host_from_tab(tab).is_some()
}

fn initial_host_from_tab(tab: &TabInfo) -> Option<String> {
    if is_internal_url(&tab.url) {
        return None;
    }

    site_from_url(&tab.url).ok()
}

fn is_internal_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("about:")
        || lower.starts_with("chrome:")
        || lower.starts_with("devtools:")
        || lower.starts_with("edge:")
}

fn is_missing_tab_error(error: &anyhow::Error) -> bool {
    let message = error_chain(error);
    message.contains("not found") || message.contains("no tab")
}

fn is_last_tab_close_error(error: &anyhow::Error) -> bool {
    error_chain(error).contains("cannot close the last tab")
}

fn error_chain(error: &anyhow::Error) -> String {
    format!("{error:?}").to_ascii_lowercase()
}

fn daemon_status(paths: &WorkspacePaths) -> String {
    if daemon_is_running(paths) {
        "running".to_string()
    } else {
        "stopped".to_string()
    }
}

fn daemon_is_running(paths: &WorkspacePaths) -> bool {
    let Ok(pid) = fs::read_to_string(paths.daemon_pid()) else {
        return false;
    };
    let Ok(pid) = pid.trim().parse::<u32>() else {
        return false;
    };

    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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
        now.second()
    )
}
