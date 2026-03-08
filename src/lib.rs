mod cli;
mod editor;
mod entry;
mod init;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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
            Commands::Import { path } => self.handle_import(path),
            Commands::File { command } => self.handle_file(command),
            Commands::Script { command } => self.handle_script(command),
            Commands::Cmd { command } => self.handle_cmd(command),
            Commands::Build { name, autobuild } => self.handle_build(name, autobuild),
            Commands::BuildAll => self.handle_build_all(),
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

    fn handle_import(&self, path: PathBuf) -> Result<()> {
        let import_root = canonicalize_scope(&path)
            .with_context(|| format!("failed to resolve import path {}", path.display()))?;
        let mut sources = discover_import_sources(&import_root)?;
        self.populate_import_dependencies(&mut sources)?;

        for source in &sources {
            validate_name(&source.name)?;
            let paths = self.entry_paths(&source.name);
            if paths.root.exists() {
                bail!(
                    "entry `{}` already exists at {}",
                    source.name,
                    paths.root.display()
                );
            }
        }

        for source in sources {
            let entry_root = self.import_entry(source)?;
            println!("{}", entry_root.display());
        }

        Ok(())
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
            shell: shell.unwrap_or_else(default_shell_path),
            depends_on: Vec::new(),
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

    fn import_entry(&self, source: ImportSource) -> Result<PathBuf> {
        let paths = self.entry_paths(&source.name);
        fs::create_dir_all(&paths.root).with_context(|| {
            format!("failed to create entry directory {}", paths.root.display())
        })?;

        let config = EntryConfig {
            name: source.name.clone(),
            workspace: source.workspace,
            managed: false,
            description: None,
            autobuild: false,
            autobuild_order: 100,
            shell: default_shell_path(),
            depends_on: source.depends_on,
        };

        write_string(&paths.config, &config.to_toml()?)?;
        match source.build_script {
            Some(build_script) => copy_file(&build_script, &paths.build_script)?,
            None => write_string(&paths.build_script, &default_build_script(&source.name))?,
        }
        write_string(&paths.run_script, &default_run_script(&source.name))?;

        Ok(paths.root)
    }

    fn list_entries(&self, autobuild_only: bool) -> Result<()> {
        for entry in self.load_entries(autobuild_only)? {
            print_entry_summary(&entry)?;
        }
        Ok(())
    }

    fn handle_build(&self, name: Option<String>, autobuild: bool) -> Result<()> {
        match (name, autobuild) {
            (Some(name), false) => {
                let entries = self.load_entries(false)?;
                self.execute_build_targets(&entries, &[name])
            }
            (None, true) => {
                let entries = self.load_entries(false)?;
                let targets = entries
                    .iter()
                    .filter(|entry| entry.config.autobuild)
                    .map(|entry| entry.config.name.clone())
                    .collect::<Vec<_>>();
                if targets.is_empty() {
                    return Ok(());
                }
                self.execute_build_targets(&entries, &targets)
            }
            (Some(_), true) => bail!("pass either a name or --autobuild, not both"),
            (None, false) => bail!("missing build target, pass <name> or --autobuild"),
        }
    }

    fn handle_build_all(&self) -> Result<()> {
        let entries = self.load_entries(false)?;
        if entries.is_empty() {
            return Ok(());
        }

        let targets = entries
            .iter()
            .map(|entry| entry.config.name.clone())
            .collect::<Vec<_>>();
        self.execute_build_targets(&entries, &targets)
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

    fn execute_script(
        &self,
        paths: &EntryPaths,
        config: &EntryConfig,
        kind: ScriptKind,
        script_path: &Path,
        execution_dir: &Path,
    ) -> Result<()> {
        let shell = resolve_shell_command(&config.shell);
        let status = Command::new(&shell)
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
                    shell.display()
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

    fn populate_import_dependencies(&self, sources: &mut [ImportSource]) -> Result<()> {
        let imported_names = sources
            .iter()
            .map(|source| source.name.clone())
            .collect::<HashSet<_>>();
        let existing_names = self
            .load_entries(false)?
            .into_iter()
            .map(|entry| entry.config.name)
            .collect::<HashSet<_>>();

        for source in sources {
            source.depends_on = infer_import_dependencies(
                &source.workspace,
                &source.name,
                &imported_names,
                &existing_names,
            )?;
        }

        Ok(())
    }

    fn execute_build_targets(&self, entries: &[EntrySummary], targets: &[String]) -> Result<()> {
        let graph = build_execution_graph(entries, targets)?;
        execute_build_graph(graph)
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

struct ImportSource {
    name: String,
    workspace: PathBuf,
    build_script: Option<PathBuf>,
    depends_on: Vec<String>,
}

#[derive(Clone)]
struct BuildNode {
    entry: EntrySummary,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    remaining_dependencies: usize,
    state: BuildNodeState,
}

#[derive(Clone)]
enum BuildNodeState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

struct BuildExecutionGraph {
    order: Vec<String>,
    nodes: HashMap<String, BuildNode>,
}

struct BuildTaskResult {
    name: String,
    duration: Duration,
    log_path: PathBuf,
    outcome: BuildTaskOutcome,
}

enum BuildTaskOutcome {
    Succeeded,
    Failed(String),
}

fn default_root() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/dmgr"))
}

fn default_shell_path() -> PathBuf {
    PathBuf::from("sh")
}

fn resolve_shell_command(shell: &Path) -> PathBuf {
    if shell.is_absolute()
        && !shell.exists()
        && let Some(name) = shell.file_name()
    {
        return PathBuf::from(name);
    }

    shell.to_path_buf()
}

fn discover_import_sources(path: &Path) -> Result<Vec<ImportSource>> {
    if !path.is_dir() {
        bail!("import path must be a directory: {}", path.display());
    }

    if is_importable_workspace(path) {
        return Ok(vec![import_source_from_workspace(path)?]);
    }

    let mut sources = Vec::new();
    for dirent in fs::read_dir(path)
        .with_context(|| format!("failed to read import root {}", path.display()))?
    {
        let dirent =
            dirent.with_context(|| format!("failed to read import root {}", path.display()))?;
        if !dirent
            .file_type()
            .with_context(|| format!("failed to inspect {}", dirent.path().display()))?
            .is_dir()
        {
            continue;
        }

        let workspace = dirent.path();
        if is_importable_workspace(&workspace) {
            sources.push(import_source_from_workspace(&workspace)?);
        }
    }

    sources.sort_by(|left, right| left.name.cmp(&right.name));
    if sources.is_empty() {
        bail!("no importable entries found under {}", path.display());
    }

    Ok(sources)
}

fn infer_import_dependencies(
    workspace: &Path,
    name: &str,
    imported_names: &HashSet<String>,
    existing_names: &HashSet<String>,
) -> Result<Vec<String>> {
    let mut depends_on = parse_dockerfile_from_images(&workspace.join("Dockerfile"))?
        .into_iter()
        .filter(|image| image != name)
        .filter(|image| imported_names.contains(image) || existing_names.contains(image))
        .collect::<Vec<_>>();
    depends_on.sort();
    depends_on.dedup();
    Ok(depends_on)
}

fn is_importable_workspace(path: &Path) -> bool {
    path.join("Dockerfile").is_file()
}

fn import_source_from_workspace(workspace: &Path) -> Result<ImportSource> {
    let name = workspace
        .file_name()
        .and_then(|name| name.to_str())
        .context("import path must end with a valid UTF-8 directory name")?
        .to_string();
    let build_script = workspace.join("build.sh");

    Ok(ImportSource {
        name,
        workspace: workspace.to_path_buf(),
        build_script: build_script.is_file().then_some(build_script),
        depends_on: Vec::new(),
    })
}

fn parse_dockerfile_from_images(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read Dockerfile {}", path.display()))?;
    let mut images = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("FROM ") {
            continue;
        }

        if let Some(image) = parse_from_image(trimmed) {
            images.push(image);
        }
    }

    Ok(images)
}

fn parse_from_image(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "FROM" {
        return None;
    }

    let mut image = None;
    for part in parts {
        if part.starts_with("--") {
            continue;
        }
        image = Some(part.to_string());
        break;
    }

    image
}

fn resolve_build_plan(entries: &[EntrySummary], targets: &[String]) -> Result<Vec<String>> {
    let mut by_name = HashMap::new();
    for (index, entry) in entries.iter().enumerate() {
        by_name.insert(entry.config.name.clone(), index);
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut plan = Vec::new();

    for target in targets {
        visit_build_dependencies(
            target,
            entries,
            &by_name,
            &mut visiting,
            &mut visited,
            &mut plan,
        )?;
    }

    Ok(plan)
}

fn build_execution_graph(
    entries: &[EntrySummary],
    targets: &[String],
) -> Result<BuildExecutionGraph> {
    let order = resolve_build_plan(entries, targets)?;
    let selected = order.iter().cloned().collect::<HashSet<_>>();
    let by_name = entries
        .iter()
        .map(|entry| (entry.config.name.clone(), entry.clone()))
        .collect::<HashMap<_, _>>();
    let mut nodes = HashMap::new();

    for name in &order {
        let entry = by_name
            .get(name)
            .with_context(|| format!("entry `{name}` not found"))?
            .clone();
        let dependencies = entry
            .config
            .depends_on
            .iter()
            .filter(|dependency| selected.contains(*dependency))
            .cloned()
            .collect::<Vec<_>>();
        let remaining_dependencies = dependencies.len();
        nodes.insert(
            name.clone(),
            BuildNode {
                entry,
                dependencies,
                dependents: Vec::new(),
                remaining_dependencies,
                state: BuildNodeState::Pending,
            },
        );
    }

    for name in &order {
        let dependencies = nodes
            .get(name)
            .map(|node| node.dependencies.clone())
            .with_context(|| format!("missing build node `{name}`"))?;
        for dependency in dependencies {
            let dependency_node = nodes
                .get_mut(&dependency)
                .with_context(|| format!("missing dependency node `{dependency}`"))?;
            dependency_node.dependents.push(name.clone());
        }
    }

    Ok(BuildExecutionGraph { order, nodes })
}

fn execute_build_graph(mut graph: BuildExecutionGraph) -> Result<()> {
    let order_index = graph
        .order
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut ready = graph
        .order
        .iter()
        .filter(|name| {
            graph
                .nodes
                .get(*name)
                .is_some_and(|node| node.remaining_dependencies == 0)
        })
        .cloned()
        .collect::<Vec<String>>();
    let max_jobs = build_jobs();
    let (tx, rx) = mpsc::channel::<BuildTaskResult>();
    let mut running = 0usize;

    while !ready.is_empty() || running > 0 {
        while running < max_jobs && !ready.is_empty() {
            let name = ready.remove(0);
            let Some(node) = graph.nodes.get_mut(&name) else {
                continue;
            };
            if !matches!(node.state, BuildNodeState::Pending) {
                continue;
            }

            node.state = BuildNodeState::Running;
            print_build_status("start", &name, None)?;

            let entry = node.entry.clone();
            let tx = tx.clone();
            thread::spawn(move || {
                let result = execute_build_task(entry);
                let _ = tx.send(result);
            });
            running += 1;
        }

        let result = rx
            .recv()
            .context("build worker channel closed unexpectedly")?;
        running -= 1;

        match result.outcome {
            BuildTaskOutcome::Succeeded => {
                if let Some(node) = graph.nodes.get_mut(&result.name) {
                    node.state = BuildNodeState::Succeeded;
                }
                print_build_status(
                    "done",
                    &result.name,
                    Some(&format_duration(result.duration)),
                )?;

                let dependents = graph
                    .nodes
                    .get(&result.name)
                    .map(|node| node.dependents.clone())
                    .unwrap_or_default();
                let mut newly_ready = Vec::new();
                for dependent in dependents {
                    let Some(node) = graph.nodes.get_mut(&dependent) else {
                        continue;
                    };
                    if !matches!(node.state, BuildNodeState::Pending) {
                        continue;
                    }
                    if node.remaining_dependencies > 0 {
                        node.remaining_dependencies -= 1;
                    }
                    if node.remaining_dependencies == 0 {
                        newly_ready.push(dependent);
                    }
                }
                insert_ready_nodes(&mut ready, newly_ready, &order_index);
            }
            BuildTaskOutcome::Failed(message) => {
                if let Some(node) = graph.nodes.get_mut(&result.name) {
                    node.state = BuildNodeState::Failed;
                }
                print_build_status(
                    "fail",
                    &result.name,
                    Some(&format!("{message}; log={}", result.log_path.display())),
                )?;

                let dependents = graph
                    .nodes
                    .get(&result.name)
                    .map(|node| node.dependents.clone())
                    .unwrap_or_default();
                skip_blocked_dependents(&mut graph.nodes, &dependents, &result.name)?;
                ready.retain(|name| {
                    graph
                        .nodes
                        .get(name)
                        .is_some_and(|node| matches!(node.state, BuildNodeState::Pending))
                });
            }
        }
    }

    print_build_summary(&graph.nodes)?;

    let failed = graph
        .nodes
        .values()
        .filter(|node| matches!(node.state, BuildNodeState::Failed))
        .count();
    let skipped = graph
        .nodes
        .values()
        .filter(|node| matches!(node.state, BuildNodeState::Skipped))
        .count();
    if failed > 0 {
        bail!("build failed: {failed} failed, {skipped} skipped");
    }

    Ok(())
}

fn visit_build_dependencies(
    name: &str,
    entries: &[EntrySummary],
    by_name: &HashMap<String, usize>,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    plan: &mut Vec<String>,
) -> Result<()> {
    if visited.contains(name) {
        return Ok(());
    }
    if !visiting.insert(name.to_string()) {
        bail!("detected build dependency cycle at `{name}`");
    }

    let &index = by_name
        .get(name)
        .with_context(|| format!("entry `{name}` not found"))?;
    let entry = &entries[index];

    for dependency in &entry.config.depends_on {
        visit_build_dependencies(dependency, entries, by_name, visiting, visited, plan)?;
    }

    visiting.remove(name);
    visited.insert(name.to_string());
    plan.push(name.to_string());
    Ok(())
}

fn execute_build_task(entry: EntrySummary) -> BuildTaskResult {
    let log_path = entry.paths.root.join("build.last.log");
    let started_at = Instant::now();
    let outcome =
        execute_build_task_inner(&entry, &log_path).unwrap_or_else(BuildTaskOutcome::Failed);

    BuildTaskResult {
        name: entry.config.name.clone(),
        duration: started_at.elapsed(),
        log_path,
        outcome,
    }
}

fn execute_build_task_inner(
    entry: &EntrySummary,
    log_path: &Path,
) -> std::result::Result<BuildTaskOutcome, String> {
    let Some(parent) = log_path.parent() else {
        return Err(format!("invalid build log path {}", log_path.display()));
    };
    fs::create_dir_all(parent)
        .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;

    let mut header = fs::File::create(log_path)
        .map_err(|err| format!("failed to create build log {}: {err}", log_path.display()))?;
    writeln!(
        header,
        "# dmgr build log\n# entry={}\n# workspace={}\n# script={}\n",
        entry.config.name,
        entry.config.workspace.display(),
        entry.paths.build_script.display()
    )
    .map_err(|err| format!("failed to write build log {}: {err}", log_path.display()))?;
    let stdout = header.try_clone().map_err(|err| {
        format!(
            "failed to clone build log handle {}: {err}",
            log_path.display()
        )
    })?;
    let stderr = header;

    let shell = resolve_shell_command(&entry.config.shell);
    let status = Command::new(&shell)
        .arg(&entry.paths.build_script)
        .current_dir(&entry.config.workspace)
        .env("DMGR_ENTRY_NAME", &entry.config.name)
        .env("DMGR_ENTRY_ROOT", &entry.paths.root)
        .env("DMGR_ENTRY_WORKSPACE", &entry.config.workspace)
        .env("DMGR_CMD_PATH", &entry.paths.build_script)
        .env("DMGR_RUN_DIR", &entry.config.workspace)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()
        .map_err(|err| {
            format!(
                "failed to execute build script for entry `{}` with shell {}: {err}",
                entry.config.name,
                shell.display()
            )
        })?;

    if status.success() {
        Ok(BuildTaskOutcome::Succeeded)
    } else {
        Ok(BuildTaskOutcome::Failed(format!("exit status {status}")))
    }
}

fn insert_ready_nodes(
    ready: &mut Vec<String>,
    mut newly_ready: Vec<String>,
    order_index: &HashMap<String, usize>,
) {
    newly_ready.sort_by_key(|name| order_index.get(name).copied().unwrap_or(usize::MAX));
    ready.extend(newly_ready);
}

fn skip_blocked_dependents(
    nodes: &mut HashMap<String, BuildNode>,
    dependents: &[String],
    blocked_by: &str,
) -> Result<()> {
    for dependent in dependents {
        let Some(node) = nodes.get_mut(dependent) else {
            continue;
        };

        if !matches!(node.state, BuildNodeState::Pending) {
            continue;
        }

        node.state = BuildNodeState::Skipped;
        print_build_status(
            "skip",
            dependent,
            Some(&format!("blocked by `{blocked_by}`")),
        )?;
        let descendants = node.dependents.clone();
        skip_blocked_dependents(nodes, &descendants, blocked_by)?;
    }

    Ok(())
}

fn print_build_status(status: &str, name: &str, detail: Option<&str>) -> Result<()> {
    let mut stdout = io::stdout().lock();
    match detail {
        Some(detail) => writeln!(stdout, "[{status}] {name} {detail}"),
        None => writeln!(stdout, "[{status}] {name}"),
    }
    .context("failed to print build status")
}

fn print_build_summary(nodes: &HashMap<String, BuildNode>) -> Result<()> {
    let succeeded = nodes
        .values()
        .filter(|node| matches!(node.state, BuildNodeState::Succeeded))
        .count();
    let failed = nodes
        .values()
        .filter(|node| matches!(node.state, BuildNodeState::Failed))
        .count();
    let skipped = nodes
        .values()
        .filter(|node| matches!(node.state, BuildNodeState::Skipped))
        .count();
    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "[summary] succeeded={succeeded} failed={failed} skipped={skipped}"
    )
    .context("failed to print build summary")
}

fn format_duration(duration: Duration) -> String {
    format!("{:.2?}", duration)
}

fn build_jobs() -> usize {
    std::env::var("DMGR_BUILD_JOBS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|jobs| *jobs > 0)
        .or_else(|| thread::available_parallelism().ok().map(usize::from))
        .unwrap_or(1)
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

fn copy_file(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    fs::copy(source, target).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
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
    format!("#!/usr/bin/env sh\nset -eu\ndocker build -t {name} -f Dockerfile .\n")
}

fn default_run_script(name: &str) -> String {
    format!("#!/usr/bin/env sh\nset -eu\ndocker run --rm -it {name}\n")
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
            shell: PathBuf::from("sh"),
            depends_on: Vec::new(),
        };

        let text = config.to_toml().expect("serialize config");
        let parsed = EntryConfig::from_toml(&text).expect("deserialize config");
        assert_eq!(parsed.shell, PathBuf::from("sh"));
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

    #[test]
    fn missing_absolute_shell_falls_back_to_program_name() {
        let resolved =
            super::resolve_shell_command(PathBuf::from("/__dmgr_missing__/sh").as_path());
        assert_eq!(resolved, PathBuf::from("sh"));
    }
}
