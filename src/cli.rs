use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "dmgr")]
#[command(version)]
#[command(about = "Manage local Docker development entries")]
pub struct Cli {
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Entry {
        #[command(subcommand)]
        command: EntryCommands,
    },
    Import {
        path: PathBuf,
    },
    File {
        #[command(subcommand)]
        command: FileCommands,
    },
    Script {
        #[command(subcommand)]
        command: ScriptCommands,
    },
    Cmd {
        #[command(subcommand)]
        command: CmdCommands,
    },
    Build {
        name: Option<String>,
        #[arg(long)]
        autobuild: bool,
    },
    BuildAll,
    Rm {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    Run {
        name: String,
    },
    Path {
        name: String,
        #[arg(value_enum, default_value_t = PathKind::Entry)]
        kind: PathKind,
    },
}

#[derive(Debug, Subcommand)]
pub enum EntryCommands {
    Create {
        name: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, default_value_t = false)]
        autobuild: bool,
        #[arg(long, default_value_t = 100)]
        autobuild_order: i32,
        #[arg(long)]
        shell: Option<PathBuf>,
    },
    List {
        #[arg(long, default_value_t = false)]
        autobuild: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum FileCommands {
    Edit { name: String },
}

#[derive(Debug, Subcommand)]
pub enum ScriptCommands {
    Edit { name: String },
}

#[derive(Debug, Subcommand)]
pub enum CmdCommands {
    Edit {
        name: String,
        #[arg(long, conflicts_with = "workspace")]
        cwd: bool,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum PathKind {
    Entry,
    Config,
    Workspace,
    File,
    Build,
    Run,
}
