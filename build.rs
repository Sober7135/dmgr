use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));

    // Cargo does not know that version output depends on git metadata unless we declare it.
    emit_git_rerun_rules(&manifest_dir);

    let pkg_version = env::var("CARGO_PKG_VERSION").expect("package version");
    let git_sha = git_sha(&manifest_dir).unwrap_or_else(|| "unknown".to_string());
    let git_commit_time = git_commit_time(&manifest_dir).unwrap_or_else(|| "unknown".to_string());
    // Keep clap's version output self-contained so `dmgr --version` exposes the exact commit.
    println!("cargo:rustc-env=DMGR_VERSION={pkg_version} ({git_sha} {git_commit_time})");
}

fn git_sha(manifest_dir: &Path) -> Option<String> {
    git_output(manifest_dir, &["rev-parse", "HEAD"])
}

fn git_commit_time(manifest_dir: &Path) -> Option<String> {
    git_output(manifest_dir, &["show", "-s", "--format=%cI", "HEAD"])
}

fn emit_git_rerun_rules(manifest_dir: &Path) {
    let head_path = git_path(manifest_dir, "HEAD");
    if let Some(head_path) = head_path {
        println!("cargo:rerun-if-changed={}", head_path.display());
    }

    let ref_path = current_ref_path(manifest_dir);
    if let Some(ref_path) = ref_path {
        println!("cargo:rerun-if-changed={}", ref_path.display());
    }
}

fn git_path(manifest_dir: &Path, path: &str) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", path])
        .current_dir(manifest_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let resolved = String::from_utf8(output.stdout).ok()?;
    let resolved = resolved.trim();
    if resolved.is_empty() {
        return None;
    }

    Some(manifest_dir.join(resolved))
}

fn current_ref_path(manifest_dir: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .current_dir(manifest_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let reference = String::from_utf8(output.stdout).ok()?;
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }

    git_path(manifest_dir, reference)
}

fn git_output(manifest_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
