//! Unix ownership of a plugin's process group, including inherited pipes.
use std::{io, os::unix::process::CommandExt, process::Command};

pub(super) fn configure(command: &mut Command) {
    command.process_group(0);
}

pub(super) struct Group(Option<libc::pid_t>);
impl Group {
    pub fn new(pid: u32) -> Self {
        Self(Some(pid.try_into().expect("Unix child PID fits pid_t")))
    }
    pub fn terminate(&mut self) -> Result<(), String> {
        let Some(pid) = self.0.take() else {
            return Ok(());
        };
        // The group was created by this task before exec; never signal the
        // editor's group. ESRCH means the complete group has already exited.
        if unsafe { libc::kill(-pid, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("终止插件进程组失败：{error}"));
            }
        }
        Ok(())
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        if let Err(error) = self.terminate() {
            log::error!("{error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };

    #[test]
    fn plugin_timeout_and_host_exit_release_descendant_pipes() {
        let directory =
            std::env::temp_dir().join(format!("mcedit-plugin-tree-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("host");
        let mut request = serde_json::from_value::<crate::RunRequest>(serde_json::json!({
            "protocol_version": crate::PROTOCOL_VERSION, "work_budget_bytes": 268435456,
            "source": "pass", "filename": "fixture.py", "package": false,
            "modules": {}, "command": null, "options": {}, "snapshot": null
        }))
        .unwrap();
        let limits = crate::RunLimits {
            timeout: Duration::from_millis(150),
            ..Default::default()
        };
        for exit in ["wait", "exit 0"] {
            std::fs::write(&executable, format!("#!/bin/sh\nsleep 20 &\n{exit}\n")).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
            let began = Instant::now();
            assert!(crate::run(&executable, &request, &limits).is_err());
            assert!(
                began.elapsed() < Duration::from_secs(5),
                "inherited pipes outlived the host"
            );
        }
        std::fs::write(&executable, "#!/bin/sh\nsleep 20 &\nwait\n").unwrap();
        request.snapshot = Some(
            serde_json::from_value(serde_json::json!({
                "format": "MCEDIT2_RUST_STRUCTURE_CLIPBOARD", "version": 2,
                "source_root": "/fixture", "dimension": "minecraft:overworld", "data_version": 3955,
                "numeric": false, "source_min": [0, 0, 0], "size": [1, 1, 1],
                "blocks": [{"name": "minecraft:stone", "properties": []}]
            }))
            .unwrap(),
        );
        request.validate().unwrap();
        let began = Instant::now();
        let error = crate::ToolRuntime::default()
            .run(&executable, &request, &limits)
            .unwrap_err();
        assert!(error.contains("超时"), "{error}");
        assert!(began.elapsed() < Duration::from_secs(5));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
