//! 候选、暂存及撤销共享已应用切面；异步读取不采用输入框中的未确认草稿。
use crate::{App, Message};
use iced::Task;
use mcedit2_world::editing::{Bounds, Slice, VerticalSlice};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
type Key = (usize, usize);
type Views = (Option<Arc<Slice>>, Option<Arc<VerticalSlice>>);
#[derive(Default)]
pub(crate) struct State {
    key: Option<Key>,
    horizontal: Option<Bounds>,
    vertical: Option<Bounds>,
}
impl State {
    pub fn remember(
        &mut self,
        key: Option<Key>,
        horizontal: Option<Bounds>,
        vertical: Option<Bounds>,
    ) -> (Option<Bounds>, Option<Bounds>) {
        if self.key != key {
            *self = Self {
                key,
                ..Default::default()
            };
        }
        if let Some(bounds) = horizontal {
            self.horizontal = Some(bounds);
        }
        if let Some(bounds) = vertical {
            self.vertical = Some(bounds);
        }
        (self.horizontal, self.vertical)
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Ready {
    key: Key,
    nonce: u64,
    token: Arc<AtomicBool>,
    result: Result<Views, String>,
}
pub(crate) fn start(app: &mut App, specs: (Option<Bounds>, Option<Bounds>)) -> Task<Message> {
    if specs == (None, None) {
        return Task::none();
    }
    let source = (|| {
        let key = app.current_key().ok_or("刷新切片时缺少世界或维度")?;
        Ok::<_, String>((key, app.display_session(key)?.clone()))
    })();
    let (key, source) = match source {
        Ok(source) => source,
        Err(error) => {
            log::error!("[map-source] {error}");
            app.slice_result = error.clone();
            app.vertical_report = error;
            return Task::none();
        }
    };
    let nonce = app.near_nonce;
    let token = Arc::clone(&app.near_cancelled);
    let worker = Arc::clone(&token);
    Task::perform(
        crate::nbt_background(move || {
            let check = || {
                if worker.load(Ordering::Acquire) {
                    Err("切片源刷新已取消".to_owned())
                } else {
                    Ok(())
                }
            };
            check()?;
            let horizontal = specs
                .0
                .map(|b| {
                    source
                        .inspect_slice(b)
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                })
                .transpose()?;
            check()?;
            let vertical = specs
                .1
                .map(|b| {
                    source
                        .inspect_vertical_slice(b)
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                })
                .transpose()?;
            check()?;
            Ok((horizontal, vertical))
        }),
        move |result| {
            Message::MapRefresh(Ready {
                key,
                nonce,
                token,
                result,
            })
        },
    )
}
pub(crate) fn update(app: &mut App, ready: Ready) {
    if app.current_key() != Some(ready.key)
        || app.near_nonce != ready.nonce
        || !Arc::ptr_eq(&app.near_cancelled, &ready.token)
        || ready.token.load(Ordering::Acquire)
    {
        log::error!("[map-source] 丢弃已退役的切片刷新结果");
        return;
    }
    match ready.result {
        Ok((horizontal, vertical)) => {
            if let Some(slice) = horizontal {
                app.slice_result = format!("已从当前显示会话刷新水平切片：{} 格。", slice.present);
                app.slice_grid = Some((ready.key.0, ready.key.1, slice));
            }
            if let Some(slice) = vertical {
                app.vertical_report = "已从当前显示会话刷新垂直切片，最高 Y 在上方。".into();
                app.vertical_grid = Some((ready.key.0, ready.key.1, slice));
            }
        }
        Err(error) => {
            log::error!("[map-source] {error}");
            app.slice_result = error.clone();
            app.vertical_report = error;
        }
    }
}
