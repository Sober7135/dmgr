use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct EntryConfig {
    pub name: String,
    #[serde(alias = "workdir")]
    pub workspace: PathBuf,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub autobuild: bool,
    #[serde(default = "default_autobuild_order")]
    pub autobuild_order: i32,
    #[serde(default = "default_shell")]
    pub shell: PathBuf,
}

impl EntryConfig {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read entry config {}", path.display()))?;
        Self::from_toml(&content).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn from_toml(content: &str) -> Result<Self> {
        toml::from_str(content).context("failed to parse entry config")
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize entry config")
    }
}

#[derive(Clone, Debug)]
pub struct EntryPaths {
    pub root: PathBuf,
    pub config: PathBuf,
    pub build_script: PathBuf,
    pub run_script: PathBuf,
}

impl EntryPaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            config: root.join("entry.toml"),
            build_script: root.join("build.sh"),
            run_script: root.join("run.sh"),
            root,
        }
    }

    pub fn workspace_root(&self) -> PathBuf {
        self.root.join("workspace")
    }

    pub fn cmd_overrides_root(&self) -> PathBuf {
        self.root.join("cmd-overrides")
    }

    pub fn dockerfile_path(&self, config: &EntryConfig) -> PathBuf {
        config.workspace.join("Dockerfile")
    }

    pub fn by_kind(&self, kind: ScriptKind, config: &EntryConfig) -> PathBuf {
        match kind {
            ScriptKind::Dockerfile => self.dockerfile_path(config),
            ScriptKind::Build => self.build_script.clone(),
            ScriptKind::Run => self.run_script.clone(),
        }
    }

    pub fn cmd_override_dir(&self, scope: &Path) -> PathBuf {
        self.cmd_overrides_root().join(encode_scope_path(scope))
    }

    pub fn cmd_override_config(&self, scope: &Path) -> PathBuf {
        self.cmd_override_dir(scope).join("scope.toml")
    }

    pub fn cmd_override_run_script(&self, scope: &Path) -> PathBuf {
        self.cmd_override_dir(scope).join("run.sh")
    }
}

#[derive(Clone, Debug)]
pub struct EntrySummary {
    pub config: EntryConfig,
    pub paths: EntryPaths,
}

impl EntrySummary {
    #[cfg(test)]
    pub fn for_test(name: &str, autobuild_order: i32, autobuild: bool) -> Self {
        let root = PathBuf::from(format!("/tmp/{name}"));
        Self {
            config: EntryConfig {
                name: name.to_string(),
                workspace: PathBuf::from("/workspace/example"),
                managed: false,
                description: None,
                autobuild,
                autobuild_order,
                shell: PathBuf::from("/bin/sh"),
            },
            paths: EntryPaths::new(root),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptKind {
    Dockerfile,
    Build,
    Run,
}

impl ScriptKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Dockerfile => "Dockerfile",
            Self::Build => "build script",
            Self::Run => "run script",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CmdOverrideConfig {
    pub path: PathBuf,
}

impl CmdOverrideConfig {
    pub fn from_path(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read cmd override config {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize cmd override config")
    }
}

fn default_shell() -> PathBuf {
    PathBuf::from("/bin/sh")
}

fn default_autobuild_order() -> i32 {
    100
}

fn encode_scope_path(scope: &Path) -> String {
    let mut encoded = String::with_capacity(scope.as_os_str().len() * 2);
    for byte in scope.as_os_str().as_encoded_bytes() {
        encoded.push(nibble_to_hex(byte >> 4));
        encoded.push(nibble_to_hex(byte & 0x0f));
    }
    encoded
}

fn nibble_to_hex(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => unreachable!("nibble must be within 0..=15"),
    }
}
