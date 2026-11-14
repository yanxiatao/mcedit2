//! 选框选项复用编辑器配置队列，世界选区与未应用草稿不写入全局设置。
use crate::App;
use mcedit2_world::{editing::SelectionShape, launch};
#[derive(Clone, Debug)]
pub(crate) struct LegacySettings {
    pub classic: Option<bool>,
    pub sticky: Option<bool>,
    pub shape: Option<SelectionShape>,
}
impl LegacySettings {
    pub(crate) fn description(&self, l: crate::language::Language) -> String {
        let mut values = Vec::new();
        let flag = |value| {
            if value {
                l.pick("开启", "On")
            } else {
                l.pick("关闭", "Off")
            }
        };
        if let Some(classic) = self.classic {
            values.push(format!("{} {}", l.pick("经典", "Classic"), flag(classic)));
        }
        if let Some(sticky) = self.sticky {
            values.push(format!("{} {}", l.pick("黏性", "Sticky"), flag(sticky)));
        }
        if let Some(shape) = self.shape {
            values.push(
                match shape {
                    SelectionShape::Box => l.pick("方盒", "Box"),
                    SelectionShape::Round => l.pick("椭球", "Round"),
                    SelectionShape::Diamond => l.pick("菱形", "Diamond"),
                    SelectionShape::Cylinder => l.pick("圆柱", "Cylinder"),
                    SelectionShape::ParabolicDome => l.pick("抛物圆顶", "Parabolic dome"),
                    SelectionShape::Chunks => l.pick("完整区块", "Whole chunks"),
                }
                .to_owned(),
            );
        }
        values.join(" · ")
    }
}
pub(crate) struct PreparedImport {
    saved: Saved,
    value: serde_json::Value,
}
impl PreparedImport {
    pub(crate) fn value(&self) -> &serde_json::Value {
        &self.value
    }
}
pub(crate) fn prepare_legacy(app: &App, legacy: &LegacySettings) -> Result<PreparedImport, String> {
    let mut saved = Saved::capture(app);
    if let Some(classic) = legacy.classic {
        saved.classic = classic;
    }
    if let Some(sticky) = legacy.sticky {
        saved.sticky = sticky;
    }
    if let Some(shape) = legacy.shape {
        saved.shape = shape;
    }
    Ok(PreparedImport {
        value: serde_json::to_value(&saved).map_err(|error| error.to_string())?,
        saved,
    })
}
/// 只安装全局交互偏好；各世界 / 结构当前选区和独立选区历史不在全局导入中改写。
pub(crate) fn install_legacy(app: &mut App, prepared: PreparedImport) {
    app.selection_edit.classic = prepared.saved.classic;
    app.selection_edit.sticky = prepared.saved.sticky;
    app.selection_edit.default_shape = prepared.saved.shape;
    app.selection_edit.preferences_report =
        "旧选框偏好已合并并加入保存队列；已有选区保持原状。".into();
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    mouse: bool,
    classic: bool,
    sticky: bool,
    shape: SelectionShape,
}
impl Saved {
    fn read(values: &serde_json::Map<String, serde_json::Value>) -> Result<Option<Self>, String> {
        values
            .get("selection")
            .map(|value| {
                let saved: Self = serde_json::from_value(value.clone())
                    .map_err(|error| format!("选框设置无效：{error}"))?;
                if saved.version != 1 {
                    return Err(format!("不支持选框设置版本 {}", saved.version));
                }
                Ok(saved)
            })
            .transpose()
    }
    fn capture(app: &App) -> Self {
        Self {
            version: 1,
            mouse: app.selection_edit.mouse_enabled,
            classic: app.selection_edit.classic,
            sticky: app.selection_edit.sticky,
            shape: app.selection_edit.default_shape,
        }
    }
}
pub(crate) fn restore(app: &mut App) {
    let result = launch::read_rust_prefs()
        .map_err(|error| error.to_string())
        .and_then(|values| Saved::read(&values));
    match result {
        Ok(Some(saved)) => {
            // 两种工具不能同时启用；本队列在一次写入中保存切换后的完整状态。
            if saved.mouse && crate::brush_stroke::enabled(app) {
                let error = "选框和笔刷设置同时启用，配置工具状态冲突";
                app.view_state
                    .optional::<()>("selection", Err(error.into()));
                app.selection_edit.preferences_report = error.into();
                return;
            }
            app.selection_edit.mouse_enabled = saved.mouse;
            app.selection_edit.classic = saved.classic;
            app.selection_edit.sticky = saved.sticky;
            app.selection_edit.default_shape = saved.shape;
            app.selection_shape = saved.shape;
        }
        Ok(None) => (),
        Err(error) => {
            app.view_state
                .optional::<()>("selection", Err(error.clone()));
            app.selection_edit.preferences_report = format!("选框设置读取失败：{error}");
            app.messages
                .push(app.selection_edit.preferences_report.clone());
        }
    }
}
pub(crate) fn remember(app: &mut App) {
    let result = serde_json::to_value(Saved::capture(app))
        .map_err(|error| error.to_string())
        .and_then(|value| app.view_state.queue_selection(value));
    app.selection_edit.preferences_report = match result {
        Ok(()) => "选框选项已加入自动保存队列".into(),
        Err(error) => {
            log::error!("[selection-preferences] {error}");
            format!("选框选项未保存：{error}")
        }
    };
}

#[cfg(test)]
pub(crate) fn run_tests() {
    let mut app = App::default();
    app.view_state = crate::view_state::State::load().unwrap().0;
    app.selection_edit.mouse_enabled = true;
    app.selection_edit.classic = true;
    app.selection_edit.sticky = true;
    app.selection_edit.default_shape = SelectionShape::Diamond;
    remember(&mut app);
    app.selection_edit.default_shape = SelectionShape::Cylinder;
    remember(&mut app);
    let expected = Saved::capture(&app);
    drop(app);
    let values = launch::read_rust_prefs().unwrap();
    assert_eq!(Saved::read(&values).unwrap().unwrap(), expected);
    assert_eq!(values["unrelated"], serde_json::json!({"保留":42}));
    assert!(values.contains_key("brush") && values.contains_key("views"));
    let mut app = App::default();
    restore(&mut app);
    assert!(
        app.selection_edit.mouse_enabled && app.selection_edit.classic && app.selection_edit.sticky
    );
    assert_eq!(app.selection_shape, SelectionShape::Cylinder);
    app.selection_edit.clear_workspace();
    assert!(app.selection_edit.mouse_enabled);
    assert_eq!(app.selection_edit.default_shape, SelectionShape::Cylinder);
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("shape", serde_json::json!("未知形状")),
    ] {
        let mut invalid = values.clone();
        invalid.get_mut("selection").unwrap()[key] = value;
        assert!(Saved::read(&invalid).is_err());
    }
    app.view_state = crate::view_state::State::load().unwrap().0;
    crate::selection_edit::deactivate_mouse(&mut app);
    assert!(!app.selection_edit.mouse_enabled);
    drop(app);
    assert!(
        !Saved::read(&launch::read_rust_prefs().unwrap())
            .unwrap()
            .unwrap()
            .mouse
    );
    let mut app = App::default();
    let _ = crate::brush_stroke::update(&mut app, crate::brush_stroke::Action::Enabled(true));
    launch::update_rust_prefs(|values| {
        values.get_mut("selection").unwrap()["mouse"] = serde_json::json!(true);
        Ok(())
    })
    .unwrap();
    restore(&mut app);
    assert!(app.selection_edit.preferences_report.contains("冲突"));
    assert!(!app.selection_edit.mouse_enabled);
}
