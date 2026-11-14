//! 本程序创建的插件进程归属；父程序退出时由操作系统关闭整组子进程。
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Child,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub(super) struct Job(OwnedHandle);
impl Job {
    pub fn new() -> Result<Self, String> {
        // 无名、不可继承的 job 仅属于当前父进程，不影响用户自行启动的程序。
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(format!(
                "无法建立插件进程归属：{}",
                std::io::Error::last_os_error()
            ));
        }
        // CreateJobObjectW 成功返回一个新的独占句柄。
        let owner = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                owner.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&information).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(format!(
                "无法配置插件进程释放规则：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(owner)
    }
    pub fn attach(&self, child: &mut Child) -> Result<(), String> {
        // 两个句柄在系统调用期间均由当前对象持有，不转移进程句柄所有权。
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            let error = std::io::Error::last_os_error();
            let killed = child.kill();
            let waited = child.wait();
            if let Err(cleanup) = killed {
                log::error!("归属登记失败后终止插件进程失败：{cleanup}");
            }
            if let Err(cleanup) = waited {
                return Err(format!(
                    "插件进程归属登记失败：{error}；回收失败：{cleanup}"
                ));
            }
            return Err(format!("插件进程归属登记失败：{error}"));
        }
        Ok(())
    }
}
