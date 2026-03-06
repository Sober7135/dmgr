use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

#[derive(Debug)]
pub struct Editor {
    program: String,
    args: Vec<String>,
}

impl Editor {
    pub fn from_env() -> Result<Self> {
        let raw = std::env::var("VISUAL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_else(|| "vi".to_string());

        Self::from_raw(&raw)
    }

    fn from_raw(raw: &str) -> Result<Self> {
        let mut parts = shell_words::split(raw)
            .with_context(|| format!("failed to parse editor command `{raw}`"))?;
        if parts.is_empty() {
            bail!("editor command must not be empty");
        }

        let program = parts.remove(0);
        Ok(Self {
            program,
            args: parts,
        })
    }

    pub fn open(&self, path: &Path) -> Result<()> {
        let status = Command::new(&self.program)
            .args(&self.args)
            .arg(path)
            .status()
            .with_context(|| format!("failed to start editor `{}`", self.program))?;

        if status.success() {
            Ok(())
        } else {
            bail!("editor `{}` exited with status {}", self.program, status)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Editor;

    #[test]
    fn parses_editor_with_arguments() {
        let editor = Editor::from_raw("code --wait").expect("parse editor");
        assert_eq!(editor.program, "code");
        assert_eq!(editor.args, vec!["--wait"]);
    }
}
