use std::path::Path;

use crate::cli::InitSystem;

pub fn render_init_script(system: InitSystem, dmgr_bin: &str, root: &Path) -> String {
    match system {
        InitSystem::Systemd => render_systemd(dmgr_bin, root),
        InitSystem::Openrc => render_openrc(dmgr_bin, root),
    }
}

fn render_systemd(dmgr_bin: &str, root: &Path) -> String {
    format!(
        "[Unit]\nDescription=dmgr autobuild\nAfter=network-online.target docker.service\nWants=network-online.target docker.service\n\n[Service]\nType=oneshot\nExecStart={} --root {} build --autobuild\n\n[Install]\nWantedBy=multi-user.target\n",
        dmgr_bin,
        root.display(),
    )
}

fn render_openrc(dmgr_bin: &str, root: &Path) -> String {
    format!(
        "#!/sbin/openrc-run\nname=\"dmgr-autobuild\"\ndescription=\"Build dmgr autobuild entries during boot\"\ncommand=\"{}\"\ncommand_args=\"build --autobuild\"\ncommand_env=\"DMGR_ROOT={}\"\npidfile=\"/run/${{RC_SVCNAME}}.pid\"\n\ncommand_background=false\n\nstart_pre() {{\n    checkpath --directory --mode 0755 /run\n}}\n\ndepend() {{\n    need docker\n    after net\n}}\n",
        dmgr_bin,
        root.display()
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::cli::InitSystem;

    use super::render_init_script;

    #[test]
    fn renders_systemd_template() {
        let rendered =
            render_init_script(InitSystem::Systemd, "/usr/bin/dmgr", Path::new("/tmp/dmgr"));
        assert!(rendered.contains("ExecStart=/usr/bin/dmgr --root /tmp/dmgr build --autobuild"));
        assert!(rendered.contains("Wants=network-online.target docker.service"));
    }

    #[test]
    fn renders_openrc_template() {
        let rendered = render_init_script(InitSystem::Openrc, "dmgr", Path::new("/tmp/dmgr"));
        assert!(rendered.contains("command=\"dmgr\""));
        assert!(rendered.contains("command_env=\"DMGR_ROOT=/tmp/dmgr\""));
    }
}
