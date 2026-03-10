use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::prelude::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

fn bin() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("dmgr"))
}

fn set_root(command: &mut Command, root: &Path) {
    command.arg("--root").arg(root);
}

fn create_workspace(tempdir: &TempDir, name: &str) -> PathBuf {
    let path = tempdir.path().join(name);
    fs::create_dir_all(&path).expect("create workspace");
    path
}

fn create_import_workspace(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::create_dir_all(&path).expect("create import workspace");
    fs::write(path.join("Dockerfile"), "FROM alpine:latest\n").expect("write Dockerfile");
    path
}

fn write_entry_config(path: &Path, content: &str) {
    fs::write(path, content).expect("write entry config");
}

fn write_script(path: &Path, content: &str) {
    fs::write(path, content).expect("write script");
    #[cfg(unix)]
    make_executable(path);
}

fn shell_path() -> PathBuf {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .into_iter()
        .flatten()
        .map(|dir| dir.join("sh"))
        .find(|path| path.is_file())
        .expect("find sh in PATH")
}

fn shell_script(body: &str) -> String {
    format!("#!{}\nset -eu\n{body}", shell_path().display())
}

fn run_with_stdin(command: &mut Command, input: &str) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn command");
    child
        .stdin
        .as_mut()
        .expect("stdin available")
        .write_all(input.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for command")
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("set executable bit");
}

#[test]
fn create_entry_writes_expected_files() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");

    let mut command = bin();
    set_root(command.arg("entry").arg("create").arg("dev"), &root);
    command.assert().success();

    let entry_root = root.join("entries").join("dev");
    assert!(entry_root.join("entry.toml").exists());
    assert!(entry_root.join("workspace").exists());
    assert!(entry_root.join("workspace/Dockerfile").exists());
    assert!(entry_root.join("build.sh").exists());
    assert!(entry_root.join("run.sh").exists());
}

#[test]
fn create_entry_uses_external_workspace_when_requested() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let workspace = create_workspace(&tempdir, "external-workspace");

    let mut command = bin();
    set_root(
        command
            .arg("entry")
            .arg("create")
            .arg("dev")
            .arg("--workspace")
            .arg(&workspace),
        &root,
    );
    command.assert().success();

    assert!(workspace.join("Dockerfile").exists());
    let config = fs::read_to_string(root.join("entries/dev/entry.toml")).expect("read config");
    assert!(config.contains("managed = false"));
    assert!(config.contains(&format!("workspace = \"{}\"", workspace.display())));
}

#[test]
fn create_entry_keeps_existing_external_dockerfile() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let workspace = create_workspace(&tempdir, "external-existing");
    let dockerfile = workspace.join("Dockerfile");
    let original = "FROM rust:latest\n";
    fs::write(&dockerfile, original).expect("write existing Dockerfile");

    let mut command = bin();
    set_root(
        command
            .arg("entry")
            .arg("create")
            .arg("dev")
            .arg("--workspace")
            .arg(&workspace),
        &root,
    );
    command.assert().success();

    let content = fs::read_to_string(&dockerfile).expect("read Dockerfile");
    assert_eq!(content, original);
}

#[test]
fn import_single_directory_creates_external_entry() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    let workspace = create_import_workspace(&import_root, "dev");
    let build_script = "#!/usr/bin/env sh\nset -eu\ndocker buildx build . --network host -t dev\n";
    fs::write(workspace.join("build.sh"), build_script).expect("write build script");

    let mut command = bin();
    set_root(command.arg("import").arg(&workspace), &root);
    command.assert().success();

    let entry_root = root.join("entries/dev");
    let config = fs::read_to_string(entry_root.join("entry.toml")).expect("read config");
    assert!(config.contains("managed = false"));
    assert!(config.contains(&format!(
        "workspace = \"{}\"",
        workspace.canonicalize().expect("canonical workspace").display()
    )));
    assert_eq!(
        fs::read_to_string(entry_root.join("build.sh")).expect("read build"),
        build_script
    );
    assert!(entry_root.join("run.sh").exists());
}

#[test]
fn import_parent_directory_scans_importable_children() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    fs::create_dir_all(&import_root).expect("create import root");

    let alpha = create_import_workspace(&import_root, "alpha");
    let beta = create_import_workspace(&import_root, "beta");
    fs::write(
        alpha.join("build.sh"),
        "#!/usr/bin/env sh\nset -eu\necho alpha\n",
    )
    .expect("write build");
    fs::create_dir_all(import_root.join("notes")).expect("create ignored dir");
    fs::write(import_root.join("notes/README.md"), "ignore\n").expect("write README");

    let mut command = bin();
    set_root(command.arg("import").arg(&import_root), &root);
    let assert = command.assert().success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout");
    let lines: Vec<_> = output.lines().collect();

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], root.join("entries/alpha").display().to_string());
    assert_eq!(lines[1], root.join("entries/beta").display().to_string());
    assert!(root.join("entries/alpha/entry.toml").exists());
    assert!(root.join("entries/beta/entry.toml").exists());
    let alpha_config =
        fs::read_to_string(root.join("entries/alpha/entry.toml")).expect("read alpha config");
    assert!(alpha_config.contains("depends_on = []"));
    assert_eq!(
        fs::read_to_string(root.join("entries/beta/build.sh")).expect("read beta build"),
        "#!/usr/bin/env sh\nset -eu\ndocker build -t beta -f Dockerfile .\n"
    );
    assert!(beta.join("Dockerfile").exists());
}

#[test]
fn import_infers_dependencies_from_dockerfile_from() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    fs::create_dir_all(&import_root).expect("create import root");

    create_import_workspace(&import_root, "dev");
    let app = import_root.join("graphar-arch-dev");
    fs::create_dir_all(&app).expect("create app workspace");
    fs::write(app.join("Dockerfile"), "FROM dev\nRUN echo app\n").expect("write Dockerfile");

    let mut command = bin();
    set_root(command.arg("import").arg(&import_root), &root);
    command.assert().success();

    let config =
        fs::read_to_string(root.join("entries/graphar-arch-dev/entry.toml")).expect("read config");
    assert!(config.contains("depends_on = [\"dev\"]"));
}

#[test]
fn build_recursively_builds_dependencies_first() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    let log = tempdir.path().join("build.log");
    fs::create_dir_all(&import_root).expect("create import root");

    let dev = create_import_workspace(&import_root, "dev");
    fs::write(
        dev.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho dev >> {}\n",
            log.display()
        ),
    )
    .expect("write dev build");

    let app = import_root.join("graphar-arch-dev");
    fs::create_dir_all(&app).expect("create app workspace");
    fs::write(app.join("Dockerfile"), "FROM dev\nRUN echo app\n").expect("write Dockerfile");
    fs::write(
        app.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho graphar-arch-dev >> {}\n",
            log.display()
        ),
    )
    .expect("write app build");

    let mut import = bin();
    set_root(import.arg("import").arg(&import_root), &root);
    import.assert().success();

    let mut build = bin();
    set_root(build.arg("build").arg("graphar-arch-dev"), &root);
    build.assert().success();

    let lines: Vec<_> = fs::read_to_string(&log)
        .expect("read build log")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines, vec!["dev", "graphar-arch-dev"]);
}

#[test]
fn autobuild_uses_dependency_order() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    let log = tempdir.path().join("autobuild.log");
    fs::create_dir_all(&import_root).expect("create import root");

    let dev = create_import_workspace(&import_root, "dev");
    fs::write(
        dev.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho dev >> {}\n",
            log.display()
        ),
    )
    .expect("write dev build");

    let app = import_root.join("graphar-arch-dev");
    fs::create_dir_all(&app).expect("create app workspace");
    fs::write(app.join("Dockerfile"), "FROM dev\nRUN echo app\n").expect("write Dockerfile");
    fs::write(
        app.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho graphar-arch-dev >> {}\n",
            log.display()
        ),
    )
    .expect("write app build");

    let mut import = bin();
    set_root(import.arg("import").arg(&import_root), &root);
    import.assert().success();

    let dev_config = root.join("entries/dev/entry.toml");
    let app_config = root.join("entries/graphar-arch-dev/entry.toml");
    write_entry_config(
        &dev_config,
        &fs::read_to_string(&dev_config)
            .expect("read dev config")
            .replace("autobuild = false", "autobuild = true"),
    );
    write_entry_config(
        &app_config,
        &fs::read_to_string(&app_config)
            .expect("read app config")
            .replace("autobuild = false", "autobuild = true"),
    );

    let mut autobuild = bin();
    set_root(autobuild.arg("build").arg("--autobuild"), &root);
    autobuild.assert().success();

    let lines: Vec<_> = fs::read_to_string(&log)
        .expect("read autobuild log")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines, vec!["dev", "graphar-arch-dev"]);
}

#[test]
fn build_all_builds_all_entries_in_dependency_order() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    let log = tempdir.path().join("build-all.log");
    fs::create_dir_all(&import_root).expect("create import root");

    let dev = create_import_workspace(&import_root, "dev");
    fs::write(
        dev.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho dev >> {}\n",
            log.display()
        ),
    )
    .expect("write dev build");

    let app = import_root.join("graphar-arch-dev");
    fs::create_dir_all(&app).expect("create app workspace");
    fs::write(app.join("Dockerfile"), "FROM dev\nRUN echo app\n").expect("write Dockerfile");
    fs::write(
        app.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho graphar-arch-dev >> {}\n",
            log.display()
        ),
    )
    .expect("write app build");

    let tools = create_import_workspace(&import_root, "tools");
    fs::write(
        tools.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho tools >> {}\n",
            log.display()
        ),
    )
    .expect("write tools build");

    let mut import = bin();
    set_root(import.arg("import").arg(&import_root), &root);
    import.assert().success();

    let mut build_all = bin();
    set_root(build_all.arg("build-all"), &root);
    build_all.assert().success();

    let lines: Vec<_> = fs::read_to_string(&log)
        .expect("read build-all log")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 3);
    let dev_index = lines
        .iter()
        .position(|line| line == "dev")
        .expect("dev built");
    let app_index = lines
        .iter()
        .position(|line| line == "graphar-arch-dev")
        .expect("app built");
    let tools_index = lines
        .iter()
        .position(|line| line == "tools")
        .expect("tools built");
    assert!(dev_index < app_index);
    assert_ne!(tools_index, app_index);
}

#[test]
fn build_all_runs_independent_entries_in_parallel() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    fs::create_dir_all(&import_root).expect("create import root");

    for name in ["alpha", "beta"] {
        let workspace = create_import_workspace(&import_root, name);
        fs::write(
            workspace.join("build.sh"),
            "#!/usr/bin/env sh\nset -eu\nsleep 1\n",
        )
        .expect("write build script");
    }

    let mut import = bin();
    set_root(import.arg("import").arg(&import_root), &root);
    import.assert().success();

    let mut build_all = bin();
    set_root(build_all.arg("build-all"), &root);
    build_all.env("DMGR_BUILD_JOBS", "2");

    let started_at = Instant::now();
    build_all.assert().success();
    let elapsed = started_at.elapsed();

    assert!(
        elapsed < Duration::from_millis(1800),
        "elapsed: {elapsed:?}"
    );
}

#[test]
fn build_output_shows_status_without_raw_build_logs() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    let workspace = create_import_workspace(&import_root, "alpha");
    fs::write(
        workspace.join("build.sh"),
        "#!/usr/bin/env sh\nset -eu\necho hidden-build-output\n",
    )
    .expect("write build script");

    let mut import = bin();
    set_root(import.arg("import").arg(&workspace), &root);
    import.assert().success();

    let mut build = bin();
    set_root(build.arg("build").arg("alpha"), &root);
    let assert = build.assert().success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout");

    assert!(output.contains("[start] alpha"));
    assert!(output.contains("[done] alpha"));
    assert!(output.contains("[summary] succeeded=1 failed=0 skipped=0"));
    assert!(!output.contains("hidden-build-output"));
}

#[test]
fn build_failure_is_reported_and_dependents_are_skipped() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let import_root = tempdir.path().join("dockerfiles");
    let success_log = tempdir.path().join("success.log");
    fs::create_dir_all(&import_root).expect("create import root");

    let dev = create_import_workspace(&import_root, "dev");
    fs::write(
        dev.join("build.sh"),
        "#!/usr/bin/env sh\nset -eu\necho hidden-failure-output\nexit 7\n",
    )
    .expect("write failing build");

    let app = import_root.join("graphar-arch-dev");
    fs::create_dir_all(&app).expect("create app workspace");
    fs::write(app.join("Dockerfile"), "FROM dev\nRUN echo app\n").expect("write Dockerfile");
    fs::write(
        app.join("build.sh"),
        "#!/usr/bin/env sh\nset -eu\necho should-not-run\n",
    )
    .expect("write app build");

    let tools = create_import_workspace(&import_root, "tools");
    fs::write(
        tools.join("build.sh"),
        format!(
            "#!/usr/bin/env sh\nset -eu\necho tools >> {}\n",
            success_log.display()
        ),
    )
    .expect("write tools build");

    let mut import = bin();
    set_root(import.arg("import").arg(&import_root), &root);
    import.assert().success();

    let mut build_all = bin();
    set_root(build_all.arg("build-all"), &root);
    build_all.env("DMGR_BUILD_JOBS", "2");
    let assert = build_all.assert().failure();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout");

    assert!(output.contains("[fail] dev"));
    assert!(output.contains("[skip] graphar-arch-dev blocked by `dev`"));
    assert!(output.contains("[summary]"));
    assert!(!output.contains("hidden-failure-output"));
    assert_eq!(
        fs::read_to_string(&success_log).expect("read success log"),
        "tools\n"
    );
    let failure_log =
        fs::read_to_string(root.join("entries/dev/build.last.log")).expect("read failure log");
    assert!(failure_log.contains("hidden-failure-output"));
}

#[test]
fn rm_deletes_managed_entry_root() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");

    let mut create = bin();
    set_root(create.arg("entry").arg("create").arg("dev"), &root);
    create.assert().success();

    let entry_root = root.join("entries/dev");
    assert!(entry_root.exists());

    let mut rm = bin();
    set_root(rm.arg("rm").arg("dev"), &root);
    let output = run_with_stdin(&mut rm, "y\n");
    assert!(output.status.success());

    assert!(!entry_root.exists());
}

#[test]
fn rm_keeps_external_workspace_contents() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let workspace = create_workspace(&tempdir, "external-rm");
    let marker = workspace.join("marker.txt");
    fs::write(&marker, "keep").expect("write marker");

    let mut create = bin();
    set_root(
        create
            .arg("entry")
            .arg("create")
            .arg("dev")
            .arg("--workspace")
            .arg(&workspace),
        &root,
    );
    create.assert().success();

    let mut rm = bin();
    set_root(rm.arg("rm").arg("dev").arg("--yes"), &root);
    rm.assert().success();

    assert!(workspace.exists());
    assert_eq!(fs::read_to_string(&marker).expect("read marker"), "keep");
    assert!(!root.join("entries/dev").exists());
}

#[test]
fn rm_aborts_without_confirmation() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");

    let mut create = bin();
    set_root(create.arg("entry").arg("create").arg("dev"), &root);
    create.assert().success();

    let entry_root = root.join("entries/dev");
    let mut rm = bin();
    set_root(rm.arg("rm").arg("dev"), &root);
    let output = run_with_stdin(&mut rm, "n\n");
    assert!(!output.status.success());

    assert!(entry_root.exists());
}

#[test]
fn edit_file_uses_editor_from_environment() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let editor_log = tempdir.path().join("editor.log");
    let editor = tempdir.path().join("editor.sh");

    fs::write(
        &editor,
        shell_script(&format!("echo \"$1\" > {}\n", editor_log.display())),
    )
    .expect("write editor");
    make_executable(&editor);

    let mut create = bin();
    set_root(create.arg("entry").arg("create").arg("dev"), &root);
    create.assert().success();

    let mut command = bin();
    set_root(command.arg("file").arg("edit").arg("dev"), &root);
    command.env_remove("VISUAL").env("EDITOR", &editor);

    command.assert().success();

    let target = fs::read_to_string(&editor_log).expect("read editor log");
    assert_eq!(
        target.trim(),
        root.join("entries/dev/workspace/Dockerfile")
            .display()
            .to_string()
    );
}

#[test]
fn cmd_edit_with_cwd_creates_override_from_default() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let scope = create_workspace(&tempdir, "project-a");
    let editor_log = tempdir.path().join("editor.log");
    let editor = tempdir.path().join("editor.sh");

    write_script(
        &editor,
        &shell_script(&format!("echo \"$1\" > {}\n", editor_log.display())),
    );

    let mut create = bin();
    set_root(create.arg("entry").arg("create").arg("dev"), &root);
    create.assert().success();

    let default_run_path = root.join("entries/dev/run.sh");
    let default_run = fs::read_to_string(&default_run_path).expect("read default run");

    let mut command = bin();
    set_root(
        command.arg("cmd").arg("edit").arg("dev").arg("--cwd"),
        &root,
    );
    command
        .env_remove("VISUAL")
        .env("EDITOR", &editor)
        .current_dir(&scope);
    command.assert().success();

    let override_root = root.join("entries/dev/cmd-overrides");
    let mut entries = fs::read_dir(&override_root)
        .expect("read override root")
        .map(|entry| entry.expect("dir entry").path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    let override_dir = entries.pop().expect("override dir");
    let override_run = override_dir.join("run.sh");
    let scope_config = override_dir.join("scope.toml");

    assert_eq!(
        fs::read_to_string(&override_run).expect("read override run"),
        default_run
    );
    assert_eq!(
        fs::read_to_string(&editor_log)
            .expect("read editor log")
            .trim(),
        override_run.display().to_string()
    );
    assert!(
        fs::read_to_string(&scope_config)
            .expect("read scope config")
            .contains(&format!(
                "path = \"{}\"",
                scope.canonicalize().expect("canonical").display()
            ))
    );
}

#[test]
fn build_autobuild_runs_entries_in_order() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let workspace_b = create_workspace(&tempdir, "workspace-b");
    let log = tempdir.path().join("build.log");
    let docker = tempdir.path().join("docker");
    let path = format!(
        "{}:{}",
        tempdir.path().display(),
        std::env::var("PATH").expect("PATH")
    );

    fs::write(
        &docker,
        shell_script(&format!(
            "printf '%s|%s\\n' \"$PWD\" \"$*\" >> {}\n",
            log.display()
        )),
    )
    .expect("write docker stub");
    make_executable(&docker);

    let mut create_beta = bin();
    set_root(
        create_beta
            .arg("entry")
            .arg("create")
            .arg("beta")
            .arg("--workspace")
            .arg(&workspace_b)
            .arg("--autobuild")
            .arg("--autobuild-order")
            .arg("20"),
        &root,
    );
    create_beta.assert().success();

    let mut create_alpha = bin();
    set_root(
        create_alpha
            .arg("entry")
            .arg("create")
            .arg("alpha")
            .arg("--autobuild")
            .arg("--autobuild-order")
            .arg("10"),
        &root,
    );
    create_alpha.assert().success();

    let mut command = bin();
    set_root(command.arg("build").arg("--autobuild"), &root);
    command.env("PATH", path);
    command.assert().success();

    let lines: Vec<_> = fs::read_to_string(&log)
        .expect("read log")
        .lines()
        .map(str::to_string)
        .collect();

    assert_eq!(lines.len(), 2);
    assert!(lines.iter().any(|line| {
        line.starts_with(&root.join("entries/alpha/workspace").display().to_string())
            && line.contains("build -t alpha -f Dockerfile .")
    }));
    assert!(lines.iter().any(|line| {
        line.starts_with(&workspace_b.display().to_string())
            && line.contains("build -t beta -f Dockerfile .")
    }));
}

#[test]
fn run_uses_default_cmd_when_no_override_matches() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let other_dir = create_workspace(&tempdir, "other-dir");
    let log = tempdir.path().join("run-default.log");

    let mut create = bin();
    set_root(create.arg("entry").arg("create").arg("dev"), &root);
    create.assert().success();

    let default_run = root.join("entries/dev/run.sh");
    write_script(
        &default_run,
        &format!(
            "#!/usr/bin/env sh\nset -eu\nprintf 'default|%s|%s\\n' \"$PWD\" \"$DMGR_RUN_DIR\" >> {}\n",
            log.display()
        ),
    );

    let mut command = bin();
    set_root(command.arg("run").arg("dev"), &root);
    command.current_dir(&other_dir);
    let assert = command.assert().success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout");

    assert!(output.contains("using cmd scope: default"));
    let workspace = root.join("entries/dev/workspace");
    let entry_root = root.join("entries/dev");
    assert_eq!(
        fs::read_to_string(&log).expect("read log"),
        format!("default|{}|{}\n", entry_root.display(), workspace.display())
    );
}

#[test]
fn run_uses_cwd_override_when_present() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let scope = create_workspace(&tempdir, "project-b");
    let log = tempdir.path().join("run-override.log");
    let editor = tempdir.path().join("editor.sh");

    write_script(
        &editor,
        &format!(
            "{}cat <<'EOF' > \"$1\"\n#!/usr/bin/env sh\nset -eu\nprintf 'override|%s|%s\\n' \"$PWD\" \"$DMGR_RUN_DIR\" >> {}\nEOF\n",
            shell_script(""),
            log.display()
        ),
    );

    let mut create = bin();
    set_root(create.arg("entry").arg("create").arg("dev"), &root);
    create.assert().success();

    let mut edit = bin();
    set_root(
        edit.arg("cmd")
            .arg("edit")
            .arg("dev")
            .arg("--workspace")
            .arg(&scope),
        &root,
    );
    edit.env_remove("VISUAL").env("EDITOR", &editor);
    edit.assert().success();

    let mut run = bin();
    set_root(run.arg("run").arg("dev"), &root);
    run.current_dir(&scope);
    let assert = run.assert().success();
    let output = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout");

    assert!(output.contains(&format!(
        "using cmd scope: {}",
        scope.canonicalize().expect("canonical").display()
    )));
    let entry_root = root.join("entries/dev");
    assert_eq!(
        fs::read_to_string(&log).expect("read log"),
        format!("override|{}|{}\n", entry_root.display(), scope.display())
    );
}

#[test]
fn init_openrc_writes_template_to_file() {
    let tempdir = TempDir::new().expect("create tempdir");
    let root = tempdir.path().join("dmgr");
    let output = tempdir.path().join("dmgr-autobuild");

    let mut command = bin();
    set_root(
        command
            .arg("init")
            .arg("openrc")
            .arg("--dmgr-bin")
            .arg("/usr/local/bin/dmgr")
            .arg("--output")
            .arg(&output),
        &root,
    );
    command.assert().success();

    let content = fs::read_to_string(&output).expect("read output");
    assert!(content.contains("command=\"/usr/local/bin/dmgr\""));
    assert!(content.contains(&format!("DMGR_ROOT={}", root.display())));
}
