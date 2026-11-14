//! 由实际原生窗口把源客户端坐标转换到最上方的本应用窗口，不移动系统光标。
use iced::{window, Point};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(crate) enum Backend {
    #[cfg(windows)]
    Win32 { hwnd: usize, application_scale: f64 },
    #[cfg(not(windows))]
    Local,
}
impl Backend {
    pub fn cross_window(&self) -> bool {
        cfg!(windows)
    }
}
pub(super) type Windows = HashMap<window::Id, Result<Backend, String>>;

pub(super) fn read(native: &dyn window::Window, application_scale: f32) -> Result<Backend, String> {
    #[cfg(windows)]
    {
        use window::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = native.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return Err("原生 Windows 窗口没有 Win32 句柄".into());
        };
        Ok(Backend::Win32 {
            hwnd: handle.hwnd.get() as usize,
            application_scale: f64::from(application_scale),
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (native, application_scale);
        Ok(Backend::Local)
    }
}

/// 系统文件投递没有随 winit 事件携带落点；在处理该原生事件时读取接收窗口的实际光标。
pub(super) fn file_point(windows: &Windows, receiver: window::Id) -> Result<Option<Point>, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::POINT, Graphics::Gdi::ScreenToClient, UI::WindowsAndMessaging::GetCursorPos,
        };
        let Backend::Win32 {
            hwnd,
            application_scale,
        } = windows
            .get(&receiver)
            .ok_or("文件投递窗口尚未登记")?
            .as_ref()
            .map_err(Clone::clone)?;
        let handle = *hwnd as _;
        let factor = scale(handle, *application_scale)?;
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return Err(format!("GetCursorPos：{}", std::io::Error::last_os_error()));
        }
        if unsafe { ScreenToClient(handle, &mut point) } == 0 {
            return Err(format!(
                "ScreenToClient：{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Some(Point::new(
            (f64::from(point.x) / factor) as f32,
            (f64::from(point.y) / factor) as f32,
        )))
    }
    #[cfg(not(windows))]
    {
        let _ = (windows, receiver);
        Ok(None)
    }
}

/// 同一窗口的本地拖放无需全局坐标；跨窗口只在已接通的原生后端上进行。
pub(super) fn target(
    windows: &Windows,
    source: window::Id,
    point: Point,
) -> Result<Option<(window::Id, Point)>, String> {
    let backend = windows
        .get(&source)
        .ok_or("拖放源窗口尚未登记")?
        .as_ref()
        .map_err(Clone::clone)?;
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::POINT,
            Graphics::Gdi::{ClientToScreen, ScreenToClient},
            UI::WindowsAndMessaging::{GetAncestor, WindowFromPoint, GA_ROOT},
        };
        let Backend::Win32 {
            hwnd,
            application_scale,
        } = backend;
        let source_handle = *hwnd as _;
        let factor = scale(source_handle, *application_scale)?;
        let coordinate = |value: f32| -> Result<i32, String> {
            let physical = (f64::from(value) * factor).round();
            if !physical.is_finite()
                || physical < f64::from(i32::MIN)
                || physical > f64::from(i32::MAX)
            {
                return Err("拖放坐标超出原生窗口范围".into());
            }
            Ok(physical as i32)
        };
        let mut screen = POINT {
            x: coordinate(point.x)?,
            y: coordinate(point.y)?,
        };
        // 句柄仅在窗口打开回执中登记，关闭请求时移除；调用始终发生在 GUI 线程。
        if unsafe { ClientToScreen(source_handle, &mut screen) } == 0 {
            return Err(format!(
                "ClientToScreen：{}",
                std::io::Error::last_os_error()
            ));
        }
        let hit = unsafe { WindowFromPoint(screen) };
        if hit.is_null() {
            return Ok(None);
        }
        let hit = unsafe { GetAncestor(hit, GA_ROOT) };
        if hit.is_null() {
            return Err(format!("GetAncestor：{}", std::io::Error::last_os_error()));
        }
        for (&id, backend) in windows {
            let Ok(Backend::Win32 {
                hwnd,
                application_scale,
            }) = backend
            else {
                continue;
            };
            if *hwnd != hit as usize {
                continue;
            }
            let factor = scale(hit, *application_scale)?;
            if unsafe { ScreenToClient(hit, &mut screen) } == 0 {
                return Err(format!(
                    "ScreenToClient：{}",
                    std::io::Error::last_os_error()
                ));
            }
            return Ok(Some((
                id,
                Point::new(
                    (f64::from(screen.x) / factor) as f32,
                    (f64::from(screen.y) / factor) as f32,
                ),
            )));
        }
        Ok(None)
    }
    #[cfg(not(windows))]
    {
        let _ = backend;
        Ok(Some((source, point)))
    }
}
#[cfg(windows)]
fn scale(
    hwnd: windows_sys::Win32::Foundation::HWND,
    application_scale: f64,
) -> Result<f64, String> {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) };
    if dpi == 0 {
        return Err("GetDpiForWindow 返回 0，无法转换拖放坐标".into());
    }
    Ok(f64::from(dpi) / 96. * application_scale)
}
pub(super) fn foreground_owned(windows: &Windows) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetAncestor, GetForegroundWindow, GA_ROOTOWNER,
        };
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_null() {
            return false;
        }
        let foreground = unsafe { GetAncestor(foreground, GA_ROOTOWNER) } as usize;
        windows.values().any(
            |backend| matches!(backend, Ok(Backend::Win32 { hwnd, .. }) if *hwnd == foreground),
        )
    }
    #[cfg(not(windows))]
    {
        let _ = windows;
        false
    }
}
