//! 只读取已安装启动器的登记位置，支持用户把 Prism 安装在非默认磁盘。
use std::path::PathBuf;

#[cfg(windows)]
fn prism_installation(current_user: bool) -> Result<Option<PathBuf>, String> {
    use std::ffi::c_void;
    #[link(name = "Advapi32")]
    extern "system" {
        fn RegGetValueW(
            key: *mut c_void,
            subkey: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            size: *mut u32,
        ) -> i32;
    }
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PrismLauncher"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name: Vec<u16> = "InstallLocation".encode_utf16().chain(Some(0)).collect();
    let key = if current_user {
        0x80000001u32
    } else {
        0x80000002u32
    };
    let key = (key as i32 as isize) as *mut c_void;
    let mut value = vec![0u16; 32768];
    let mut size = (value.len() * 2) as u32;
    // RRF_RT_REG_SZ，仅接受普通路径字符串，不展开环境变量或运行命令。
    let status = unsafe {
        RegGetValueW(
            key,
            subkey.as_ptr(),
            name.as_ptr(),
            2,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut size,
        )
    };
    match status {
        2 | 3 => Ok(None),
        0 => {
            if !size.is_multiple_of(2) || size as usize > value.len() * 2 {
                return Err("Prism 安装位置的注册表长度无效".into());
            }
            let end = value[..size as usize / 2]
                .iter()
                .position(|v| *v == 0)
                .unwrap_or(size as usize / 2);
            if end == 0 {
                return Ok(None);
            }
            let path = String::from_utf16(&value[..end])
                .map_err(|e| format!("Prism 安装位置无效：{e}"))?;
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("Prism 安装位置必须是绝对路径".into());
            }
            Ok(Some(path))
        }
        other => Err(format!(
            "无法读取 Prism 安装位置：{}",
            std::io::Error::from_raw_os_error(other)
        )),
    }
}

pub(crate) fn installed() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut paths = Vec::new();
        for current_user in [true, false] {
            match prism_installation(current_user) {
                Ok(Some(path)) if !paths.contains(&path) => paths.push(path),
                Ok(_) => (),
                Err(error) => log::error!("{error}"),
            }
        }
        paths
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
