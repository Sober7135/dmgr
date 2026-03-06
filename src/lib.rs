mod cli;
mod editor;
mod entry;
mod init;

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Parser;

use crate::cli::{
    Cli, CmdCommands, Commands, EntryCommands, FileCommands, InitSystem, PathKind, ScriptCommands,
};
use crate::editor::Editor;
use crate::entry::{CmdOverrideConfig, EntryConfig, EntryPaths, EntrySummary, ScriptKind};
use crate::init::render_init_script;

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let app = App::new(cli.root)?;
    app.dispatch(cli.command)
}

struct App {
    root: PathBuf,
}

impl App {
    fn new(root: Option<PathBuf>) -> Result<Self> {
        let root = match root {
            Some(root) => root,
            None => std::env::var_os("DMGR_ROOT")
                .map(PathBuf::from)
                .or_else(default_root)
                .context("failed to determine dmgr root, pass --root or set DMGR_ROOT")?,
        };

        Ok(Self { root })
    }

    fn dispatch(&self, command: Commands) -> Result<()> {
        match command {
            Commands::Entry { command } => self.handle_entry(command),
            Commands::File { command } => self.handle_file(command),
            Commands::Script { command } => self.handle_script(command),
            Commands::Cmd { command } => self.handle_cmd(command),
            Commands::Build { name, autobuild } => self.handle_build(name, autobuild),
            Commands::Rm { name, yes } => self.handle_rm(&name, yes),
            Commands::Run { name } => self.handle_run(&name),
            Commands::Path { name, kind } => self.handle_path(&name, kind),
            Commands::Init {
                system,
                output,
                dmgr_bin,
            } => self.handle_init(system, output, dmgr_bin),
        }
    }

    fn handle_entry(&self, command: EntryCommands) -> Result<()> {
        match command {
            EntryCommands::Create {
                name,
                workspace,
                description,
                autobuild,
                autobuild_order,
                shell,
            } => self.create_entry(
                &name,
                workspace,
                description,
                autobuild,
                autobuild_order,
                shell,
            ),
            EntryCommands::List { autobuild } => self.list_entries(autobuild),
        }
    }

    fn handle_file(&self, command: FileCommands) -> Result<()> {
        match command {
            FileCommands::Edit { name } => self.open_entry_path(&name, ScriptKind::Dockerfile),
        }
    }

    fn handle_script(&self, command: ScriptCommands) -> Result<()> {
        match command {
            ScriptCommands::Edit { name } => self.open_entry_path(&name, ScriptKind::Build),
        }
    }

    fn handle_cmd(&self, command: CmdCommands) -> Result<()> {
        match command {
            CmdCommands::Edit {
                name,
                cwd,
                workspace,
            } => self.open_cmd_path(&name, cwd, workspace),
        }
    }

    fn create_entry(
        &self,
        name: &str,
        workspace: Option<PathBuf>,
        description: Option<String>,
        autobuild: bool,
        autobuild_order: i32,
        shell: Option<PathBuf>,
    ) -> Result<()> {
        validate_name(name)?;

        let paths = self.entry_paths(name);
        if paths.root.exists() {
            bail!("entry `{name}` already exists at {}", paths.root.display());
        }

        fs::create_dir_all(&paths.root).with_context(|| {
            format!("failed to create entry directory {}", paths.root.display())
        })?;

        let (workspace, managed) = match workspace {
            Some(workspace) => (workspace, false),
            None => (paths.workspace_root(), true),
        };

        let config = EntryConfig {
            name: name.to_string(),
            workspace,
            managed,
            description,
            autobuild,
            autobuild_order,
            shell: shell.unwrap_or_else(|| PathBuf::from("/bin/sh")),
        };

        fs::create_dir_all(&config.workspace).with_context(|| {
            format!("failed to create workspace {}", config.workspace.display())
        })?;

        write_string(&paths.config, &config.to_toml()?)?;
        write_default_if_missing(&paths.dockerfile_path(&config), &default_dockerfile())?;
        write_string(&paths.build_script, &default_build_script(name))?;
        write_string(&paths.run_script, &default_run_script(name))?;

        println!("{}", paths.root.display());
        Ok(())
    }

    fn list_entries(&self, autobuild_only: bool) -> Result<()> {
        for entry in self.load_entries(autobuild_only)? {
            print_entry_summary(&entry)?;
        }
        Ok(())
    }

    fn handle_build(&self, name: Option<String>, autobuild: bool) -> Result<()> {
        match (name, autobuild) {
            (Some(name), false) => self.execute_entry_script(&name, ScriptKind::Build),
            (None, true) => {
                for entry in self.load_entries(true)? {
                    self.execute_script(
                        &entry.paths,
                        &entry.config,
                        ScriptKind::Build,
                        &entry.paths.build_script,
                        &entry.config.workspace,
                    )?;
                }
                Ok(())
            }
            (Some(_), true) => bail!("pass either a name or --autobuild, not both"),
            (None, false) => bail!("missing build target, pass <name> or --autobuild"),
        }
    }

    fn handle_run(&self, name: &str) -> Result<()> {
        let paths = self.entry_paths(name);
        ensure_entry_exists(&paths)?;
        let config = EntryConfig::from_path(&paths.config)?;
        let target = self.resolve_run_target(&paths, &config)?;
        println!("using cmd scope: {}", target.scope_label);
        self.execute_script(
            &paths,
            &config,
            ScriptKind::Run,
            &target.script_path,
            &target.execution_dir,
        )
    }

    fn handle_rm(&self, name: &str, yes: bool) -> Result<()> {
        let paths = self.entry_paths(name);
        ensure_entry_exists(&paths)?;
        let config = EntryConfig::from_path(&paths.config)?;
        if !yes {
            confirm_rm(&paths, &config)?;
        }
        fs::remove_dir_all(&paths.root).with_context(|| {
            format!("failed to remove entry directory {}", paths.root.display())
        })?;
        println!("{}", paths.root.display());
        Ok(())
    }

    fn handle_path(&self, name: &str, kind: PathKind) -> Result<()> {
        let paths = self.entry_paths(name);
        let config = EntryConfig::from_path(&paths.config)?;
        match kind {
            PathKind::Entry => println!("{}", paths.root.display()),
            PathKind::Config => println!("{}", paths.config.display()),
            PathKind::Workspace => println!("{}", config.workspace.display()),
            PathKind::File => println!("{}", paths.dockerfile_path(&config).display()),
            PathKind::Build => println!("{}", paths.build_script.display()),
            PathKind::Run => println!("{}", paths.run_script.display()),
        }
        Ok(())
    }

    fn handle_init(
        &self,
        system: InitSystem,
        output: Option<PathBuf>,
        dmgr_bin: String,
    ) -> Result<()> {
        let content = render_init_script(system, &dmgr_bin, &self.root);
        match output {
            Some(path) => write_string(&path, &content)?,
            None => {
                let mut stdout = io::stdout().lock();
                stdout
                    .write_all(content.as_bytes())
                    .context("failed to write init template to stdout")?;
            }
        }
        Ok(())
    }

    fn open_entry_path(&self, name: &str, kind: ScriptKind) -> Result<()> {
        let paths = self.entry_paths(name);
        ensure_entry_exists(&paths)?;
        let config = EntryConfig::from_path(&paths.config)?;
        let editor = Editor::from_env()?;
        let target = paths.by_kind(kind, &config);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        if kind == ScriptKind::Dockerfile && !target.exists() {
            write_string(&target, &default_dockerfile())?;
        }
        editor.open(&target)
    }

    fn open_cmd_path(&self, name: &str, cwd: bool, workspace: Option<PathBuf>) -> Result<()> {
        let paths = self.entry_paths(name);
        ensure_entry_exists(&paths)?;
        let editor = Editor::from_env()?;

        match resolve_cmd_scope(cwd, workspace)? {
            Some(scope) => {
                let target = self.ensure_cmd_override(&paths, &scope)?;
                editor.open(&target)
            }
            None => editor.open(&paths.run_script),
        }
    }

    fn execute_entry_script(&self, name: &str, kind: ScriptKind) -> Result<()> {
        let paths = self.entry_paths(name);
        ensure_entry_exists(&paths)?;
        let config = EntryConfig::from_path(&paths.config)?;
        let script_path = paths.by_kind(kind, &config);
        self.execute_script(&paths, &config, kind, &script_path, &config.workspace)
    }

    fn execute_script(
        &self,
        paths: &EntryPaths,
        config: &EntryConfig,
        kind: ScriptKind,
        script_path: &Path,
        execution_dir: &Path,
    ) -> Result<()> {
        let status = Command::new(&config.shell)
            .arg(script_path)
            .current_dir(execution_dir)
            .env("DMGR_ENTRY_NAME", &config.name)
            .env("DMGR_ENTRY_ROOT", &paths.root)
            .env("DMGR_ENTRY_WORKSPACE", &config.workspace)
            .env("DMGR_CMD_PATH", script_path)
            .env("DMGR_RUN_DIR", execution_dir)
            .status()
            .with_context(|| {
                format!(
                    "failed to execute {} for entry `{}` with shell {}",
                    kind.label(),
                    config.name,
                    config.shell.display()
                )
            })?;

        if status.success() {
            Ok(())
        } else {
            bail!(
                "{} for entry `{}` exited with status {}",
                kind.label(),
                config.name,
                status
            )
        }
    }

    fn load_entries(&self, autobuild_only: bool) -> Result<Vec<EntrySummary>> {
        let entries_dir = self.root.join("entries");
        if !entries_dir.exists() {
            return Ok(Vec::new());
        }

        let mut summaries = Vec::new();
        for dirent in fs::read_dir(&entries_dir)
            .with_context(|| format!("failed to read {}", entries_dir.display()))?
        {
            let dirent =
                dirent.with_context(|| format!("failed to read {}", entries_dir.display()))?;
            if !dirent
                .file_type()
                .with_context(|| format!("failed to inspect {}", dirent.path().display()))?
                .is_dir()
            {
                continue;
            }

            let paths = EntryPaths::new(dirent.path());
            if !paths.config.exists() {
                continue;
            }

            let config = EntryConfig::from_path(&paths.config)?;
            if autobuild_only && !config.autobuild {
                continue;
            }

            summaries.push(EntrySummary { config, paths });
        }

        summaries.sort_by(|left, right| {
            left.config
                .autobuild_order
                .cmp(&right.config.autobuild_order)
                .then_with(|| left.config.name.cmp(&right.config.name))
        });
        Ok(summaries)
    }

    fn entry_paths(&self, name: &str) -> EntryPaths {
        EntryPaths::new(self.root.join("entries").join(name))
    }

    fn ensure_cmd_override(&self, paths: &EntryPaths, scope: &Path) -> Result<PathBuf> {
        let target = paths.cmd_override_run_script(scope);
        if target.exists() {
            return Ok(target);
        }

        let config = CmdOverrideConfig {
            path: scope.to_path_buf(),
        };
        write_string(&paths.cmd_override_config(scope), &config.to_toml()?)?;
        let default_run = fs::read_to_string(&paths.run_script)
            .with_context(|| format!("failed to read {}", paths.run_script.display()))?;
        write_string(&target, &default_run)?;
        Ok(target)
    }

    fn resolve_run_target(
        &self,
        paths: &EntryPaths,
        config: &EntryConfig,
    ) -> Result<ResolvedRunTarget> {
        let current_dir =
            std::env::current_dir().context("failed to determine current directory")?;
        let scope = canonicalize_scope(&current_dir).with_context(|| {
            format!(
                "failed to resolve current directory {}",
                current_dir.display()
            )
        })?;

        let config_path = paths.cmd_override_config(&scope);
        let script_path = paths.cmd_override_run_script(&scope);
        if config_path.exists() && script_path.exists() {
            let override_config = CmdOverrideConfig::from_path(&config_path)?;
            if override_config.path == scope {
                return Ok(ResolvedRunTarget {
                    script_path,
                    scope_label: scope.display().to_string(),
                    execution_dir: scope,
                });
            }
        }

        Ok(ResolvedRunTarget {
            script_path: paths.run_script.clone(),
            scope_label: "default".to_string(),
            execution_dir: config.workspace.clone(),
        })
    }
}

struct ResolvedRunTarget {
    script_path: PathBuf,
    scope_label: String,
    execution_dir: PathBuf,
}

fn default_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/dmgr"))
}

fn confirm_rm(paths: &EntryPaths, config: &EntryConfig) -> Result<()> {
    let workspace_mode = if config.managed {
        "managed"
    } else {
        "external"
    };
    let workspace_note = if config.managed {
        "workspace will be removed with the entry"
    } else {
        "external workspace will be preserved"
    };

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "Remove entry `{}`?", config.name).context("failed to write prompt")?;
    writeln!(stdout, "  entry: {}", paths.root.display()).context("failed to write prompt")?;
    writeln!(
        stdout,
        "  workspace: {} ({workspace_mode}, {workspace_note})",
        config.workspace.display()
    )
    .context("failed to write prompt")?;
    write!(stdout, "Continue? [y/N] ").context("failed to write prompt")?;
    stdout.flush().context("failed to flush prompt")?;

    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read confirmation")?;

    if matches!(answer.trim(), "y" | "Y" | "yes" | "YES" | "Yes") {
        return Ok(());
    }

    bail!("aborted")
}

fn ensure_entry_exists(paths: &EntryPaths) -> Result<()> {
    if paths.config.exists() {
        return Ok(());
    }

    bail!("entry not found at {}", paths.root.display())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("entry name must not be empty");
    }

    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("entry name `{name}` contains unsupported characters");
    }

    Ok(())
}

fn write_string(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

fn write_default_if_missing(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    write_string(path, content)
}

fn resolve_cmd_scope(cwd: bool, workspace: Option<PathBuf>) -> Result<Option<PathBuf>> {
    match (cwd, workspace) {
        (true, Some(_)) => bail!("pass either --cwd or --workspace, not both"),
        (true, None) => Ok(Some(
            canonicalize_scope(
                &std::env::current_dir().context("failed to determine current directory")?,
            )
            .context("failed to resolve current directory")?,
        )),
        (false, Some(workspace)) => {
            Ok(Some(canonicalize_scope(&workspace).with_context(|| {
                format!("failed to resolve workspace {}", workspace.display())
            })?))
        }
        (false, None) => Ok(None),
    }
}

fn canonicalize_scope(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("failed to canonicalize {}", path.display()))
}

fn default_dockerfile() -> String {
    "FROM alpine:latest\n".to_string()
}

fn default_build_script(name: &str) -> String {
    format!("#!/bin/sh\nset -eu\ndocker build -t {name} -f Dockerfile .\n")
}

fn default_run_script(name: &str) -> String {
    format!("#!/bin/sh\nset -eu\ndocker run --rm -it {name}\n")
}

fn print_entry_summary(entry: &EntrySummary) -> Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "{}\t{}\tmanaged={}\tautobuild={}\torder={}",
        entry.config.name,
        entry.config.workspace.display(),
        entry.config.managed,
        entry.config.autobuild,
        entry.config.autobuild_order
    )
    .context("failed to print entry summary")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::entry::{EntryConfig, EntrySummary};

    use super::App;

    #[test]
    fn sorts_autobuild_entries_by_order_then_name() {
        let root = PathBuf::from("/tmp/example");
        let app = App { root };
        let left = EntrySummary::for_test("beta", 20, true);
        let middle = EntrySummary::for_test("alpha", 20, true);
        let right = EntrySummary::for_test("gamma", 10, true);
        let mut entries = vec![left, middle, right];

        entries.sort_by(|left, right| {
            left.config
                .autobuild_order
                .cmp(&right.config.autobuild_order)
                .then_with(|| left.config.name.cmp(&right.config.name))
        });

        let names: Vec<_> = entries.into_iter().map(|entry| entry.config.name).collect();
        assert_eq!(names, vec!["gamma", "alpha", "beta"]);
        assert_eq!(app.root, PathBuf::from("/tmp/example"));
    }

    #[test]
    fn default_config_has_shell() {
        let config = EntryConfig {
            name: "dev".to_string(),
            workspace: PathBuf::from("/workspace/dev"),
            managed: true,
            description: None,
            autobuild: false,
            autobuild_order: 100,
            shell: PathBuf::from("/bin/sh"),
        };

        let text = config.to_toml().expect("serialize config");
        let parsed = EntryConfig::from_toml(&text).expect("deserialize config");
        assert_eq!(parsed.shell, PathBuf::from("/bin/sh"));
    }

    #[test]
    fn legacy_workdir_still_deserializes_as_workspace() {
        let content = r#"
name = "dev"
workdir = "/workspace/dev"
autobuild = true
autobuild_order = 20
shell = "/bin/sh"
"#;

        let parsed = EntryConfig::from_toml(content).expect("deserialize config");
        assert_eq!(parsed.workspace, PathBuf::from("/workspace/dev"));
        assert!(!parsed.managed);
    }
}
