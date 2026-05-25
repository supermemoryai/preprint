use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "preprint")]
#[command(about = "Expose live browser sessions as editable markdown files")]
pub struct Cli {
    #[arg(long, global = true, default_value = ".")]
    pub workspace: PathBuf,

    #[arg(long, global = true)]
    pub dev: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Init,
    Open {
        url: String,

        #[arg(long)]
        session: Option<String>,

        #[arg(long)]
        context: Option<String>,

        #[arg(long, alias = "headed")]
        preview: bool,

        #[arg(long)]
        inject: bool,

        #[arg(long)]
        no_daemon: bool,

        #[arg(long, default_value_t = 750)]
        poll_ms: u64,

        #[arg(long, num_args = 0..=1, default_missing_value = "", conflicts_with = "no_profile")]
        profile: Option<String>,

        #[arg(long, conflicts_with = "profile")]
        no_profile: bool,
    },
    Daemon {
        #[arg(long, default_value_t = 750)]
        poll_ms: u64,

        #[arg(long)]
        once: bool,

        #[arg(long)]
        inject: bool,
    },
    Sync {
        tab_key: Option<String>,
    },
    Close {
        tab_key: String,
    },
    Status,
    Stop,
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum SkillCommands {
    Install,
}
