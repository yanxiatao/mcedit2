//! 资源数量不设上限；按可用物理内存或用户指定的工作预算检查分配。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResourceBudget {
    pub bytes: u64,
}
impl ResourceBudget {
    pub fn from_input(input: &str) -> Result<Self, String> {
        if input.trim().is_empty() {
            return Self::automatic();
        }
        let mib: u64 = input
            .trim()
            .parse()
            .map_err(|_| "资源预算必须是正整数 MiB，留空为自动")?;
        let bytes = mib
            .checked_mul(1024 * 1024)
            .filter(|n| *n > 0 && *n <= isize::MAX as u64)
            .ok_or("资源预算超出地址范围")?;
        Ok(Self { bytes })
    }
    pub fn automatic() -> Result<Self, String> {
        // 留出一半当前可用内存给世界网格、现有图集与其他应用。不是物理内存总量。
        let bytes = (available_memory()? / 2).min(isize::MAX as u64);
        if bytes == 0 {
            return Err("没有可用资源内存；请关闭其他程序或指定资源预算".into());
        }
        Ok(Self { bytes })
    }
    pub fn check(self, model_source: usize, resident: usize, atlas: u64) -> Result<(), String> {
        // 模型 JSON / OBJ / MTL 的解析结构按源大小八倍预留；图集计 CPU 与 GPU 各一份。
        // 这是分配前的工作集估算，不声称等于进程 RSS / 驱动实际显存。
        // resident 包含当前阶段的资源索引、目录排序与解码 RGBA。
        let needed = (model_source as u64)
            .checked_mul(8)
            .and_then(|n| n.checked_add(resident as u64))
            .and_then(|n| atlas.checked_mul(2).and_then(|a| n.checked_add(a)))
            .ok_or("资源工作集大小溢出")?;
        if needed > self.bytes {
            return Err(format!(
                "资源工作集预计需要 {} MiB，超过当前 {} MiB 预算；请提高资源内存预算后重新加载。",
                needed.div_ceil(1024 * 1024),
                self.bytes / (1024 * 1024)
            ));
        }
        Ok(())
    }
}

/// 静态 UV 分割复用资源加载预算；数量由源 UV 推导，不另设面数门槛。
/// 无预算的合成数据仍受实际地址表示范围约束。
pub(crate) fn check_expansion<T>(items: f64, budget: Option<ResourceBudget>) -> Result<(), String> {
    let bytes = items * std::mem::size_of::<T>() as f64;
    if !bytes.is_finite() || bytes < 0. || bytes > isize::MAX as f64 {
        return Err("模型 UV 扩展超出地址范围".into());
    }
    if let Some(budget) = budget {
        budget.check(0, bytes.ceil() as usize, 0)?;
    }
    Ok(())
}

#[cfg(windows)]
fn available_memory() -> Result<u64, String> {
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page: u64,
        available_page: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }
    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        load: 0,
        total_physical: 0,
        available_physical: 0,
        total_page: 0,
        available_page: 0,
        total_virtual: 0,
        available_virtual: 0,
        available_extended: 0,
    };
    // SAFETY: repr(C) 与 MEMORYSTATUSEX 一致，长度初始化，指针指向可写的完整结构。
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(format!(
            "读取可用物理内存失败：{}；请显式指定资源预算",
            std::io::Error::last_os_error()
        ));
    }
    Ok(status.available_physical)
}
#[cfg(target_os = "linux")]
fn available_memory() -> Result<u64, String> {
    crate::linux_memory::available()
}
#[cfg(not(any(windows, target_os = "linux")))]
fn available_memory() -> Result<u64, String> {
    Err("此平台尚未提供可用内存查询；请在资源面板显式指定 MiB 预算".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_working_set_is_checked_and_can_be_raised() {
        let small = ResourceBudget::from_input("1").unwrap();
        assert!(small.check(100_000, 200_000, 50_000).is_err());
        assert!(ResourceBudget::from_input("2")
            .unwrap()
            .check(100_000, 200_000, 50_000)
            .is_ok());
        for bad in ["0", "-1", "NaN", "18446744073709551615"] {
            assert!(ResourceBudget::from_input(bad).is_err());
        }
        assert!(ResourceBudget::from_input("8192")
            .unwrap()
            .check(300 * 1024 * 1024, 600 * 1024 * 1024, 1200 * 1024 * 1024)
            .is_ok());
    }
    #[test]
    #[cfg(any(windows, target_os = "linux"))]
    #[cfg(any(windows, target_os = "linux"))]
    fn automatic_queries_available_memory() {
        assert!(ResourceBudget::automatic().unwrap().bytes > 0);
    }
}
