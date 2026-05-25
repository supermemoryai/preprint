mod action;
mod agent_browser;
mod binary;
mod cli;
mod console_tail;
mod daemon;
mod diff;
mod document;
mod inject;
mod paths;
mod profile;
mod session;

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;
use tracing::Instrument;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    EnvFilter, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};

use agent_browser::AgentBrowser;
use cli::{Cli, Commands, SkillCommands};
use daemon::{DaemonOptions, OpenOptions};
use paths::WorkspacePaths;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let workspace = resolve_workspace(cli.workspace)?;
    let paths = WorkspacePaths::new(workspace)?;
    let dev = cli.dev;
    let _telemetry_guard = if dev && command_writes_runtime(&cli.command) {
        init_telemetry(&paths.logs_dir())
    } else {
        None
    };
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        workspace = %paths.root().display(),
        "preprint starting"
    );

    match cli.command {
        Commands::Init => {
            paths.ensure()?;
            tracing::info!("workspace initialized");
            println!("preprint ready");
            Ok(())
        }
        Commands::Open {
            url,
            session,
            context,
            preview,
            inject,
            no_daemon,
            poll_ms,
            profile,
            no_profile,
        } => {
            let (session_name, profile_for_launch) =
                resolve_session_and_profile(session, profile, no_profile);
            let agent_browser = agent_browser_for(&paths).await?;
            let span = tracing::info_span!("cmd.open", url = %url, session = %session_name);
            async move {
                daemon::open(
                    &paths,
                    &agent_browser,
                    OpenOptions {
                        url,
                        session: session_name,
                        context,
                        preview,
                        inject,
                        spawn_daemon: !no_daemon,
                        poll_ms,
                        dev,
                        profile: profile_for_launch,
                    },
                )
                .await
            }
            .instrument(span)
            .await
        }
        Commands::Daemon {
            poll_ms,
            once,
            inject,
        } => {
            let agent_browser = agent_browser_for(&paths).await?;
            let span = tracing::info_span!("cmd.daemon", poll_ms);
            async move {
                daemon::run(
                    &paths,
                    &agent_browser,
                    DaemonOptions {
                        poll_ms,
                        once,
                        inject,
                    },
                )
                .await
            }
            .instrument(span)
            .await
        }
        Commands::Sync { tab_key } => {
            let agent_browser = agent_browser_for(&paths).await?;
            let span = tracing::info_span!("cmd.sync", tab_key = ?tab_key);
            async move { daemon::sync(&paths, &agent_browser, tab_key).await }
                .instrument(span)
                .await
        }
        Commands::Close { tab_key } => {
            let agent_browser = agent_browser_for(&paths).await?;
            let span = tracing::info_span!("cmd.close", tab_key = %tab_key);
            async move { daemon::close_tab(&paths, &agent_browser, &tab_key).await }
                .instrument(span)
                .await
        }
        Commands::Status => {
            let _span = tracing::info_span!("cmd.status").entered();
            daemon::status(&paths)
        }
        Commands::Stop => {
            let agent_browser = agent_browser_for(&paths).await?;
            let span = tracing::info_span!("cmd.stop");
            async move { daemon::stop(&paths, &agent_browser).await }
                .instrument(span)
                .await
        }
        Commands::Skill { command } => match command {
            SkillCommands::Install => {
                let _span = tracing::info_span!("cmd.skill.install").entered();
                inject::write_skill_file(&paths)?;
                println!("skill installed at {}", paths.skill_md().display());
                Ok(())
            }
        },
    }
}

async fn agent_browser_for(paths: &WorkspacePaths) -> anyhow::Result<AgentBrowser> {
    let binary = binary::resolve_agent_browser().await?;
    Ok(AgentBrowser::new(paths.root().to_path_buf(), binary))
}

fn command_writes_runtime(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Init
            | Commands::Open { .. }
            | Commands::Daemon { .. }
            | Commands::Sync { .. }
            | Commands::Close { .. }
    )
}

fn resolve_workspace(path: PathBuf) -> anyhow::Result<PathBuf> {
    if path.exists() {
        path.canonicalize()
            .with_context(|| format!("failed to resolve workspace {}", path.display()))
    } else {
        anyhow::bail!("workspace does not exist: {}", path.display())
    }
}

fn resolve_session_and_profile(
    session_arg: Option<String>,
    profile_arg: Option<String>,
    no_profile: bool,
) -> (String, Option<String>) {
    if no_profile {
        let session_name = session_arg.unwrap_or_else(|| "no-profile".to_string());
        return (session_name, None);
    }

    let default = profile::detect_default_profile_dir();

    let profile_for_launch: Option<String> = match profile_arg {
        Some(ref p) if !p.is_empty() => Some(
            profile::resolve_profile_dir(p).unwrap_or_else(|| p.clone()),
        ),
        _ => default.clone(),
    };

    let session_name = match session_arg {
        Some(s) => s,
        None => match (&profile_for_launch, &default) {
            (Some(p), Some(d)) if p == d => "default".to_string(),
            (Some(p), _) => profile::slug_for_session(p),
            (None, _) => "default".to_string(),
        },
    };

    if profile_for_launch.is_none() && !no_profile {
        eprintln!(
            "warning: could not detect default Chrome profile; opening blank Chromium. \
             Pass --profile <name> or --no-profile to silence."
        );
    }

    (session_name, profile_for_launch)
}

fn init_telemetry(logs_dir: &Path) -> Option<WorkerGuard> {
    if let Err(error) = std::fs::create_dir_all(logs_dir) {
        eprintln!(
            "preprint: could not create logs directory {}: {error}. continuing without telemetry.",
            logs_dir.display()
        );
        return None;
    }

    let appender = tracing_appender::rolling::daily(logs_dir, "preprint.jsonl");
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("preprint=info,agent_browser=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_span_events(FmtSpan::CLOSE)
                .json(),
        )
        .init();

    Some(guard)
}
