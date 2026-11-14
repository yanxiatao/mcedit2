//! 笔刷参数与现有视图共用配置队列；保存有效参数，未完成的输入留在当前面板。
use super::*;
use mcedit2_world::{editing::BiomeTarget, launch};

/// 旧设置只持久化模式、形状和尺寸；不带方块、群系、规则或启用状态。
#[derive(Clone, Debug)]
pub(crate) struct LegacySettings {
    pub mode: Option<Mode>,
    pub shape: Option<Option<BrushShape>>,
    pub size: Option<[u16; 3]>,
}
impl LegacySettings {
    pub(crate) fn description(&self, l: crate::language::Language) -> String {
        let mut parts = Vec::new();
        if let Some(mode) = self.mode {
            parts.push(
                match mode {
                    Mode::Fill => l.pick("填充", "Fill"),
                    Mode::Replace => l.pick("替换", "Replace"),
                    Mode::Biome => l.pick("群系", "Biome"),
                }
                .to_owned(),
            );
        }
        if let Some(shape) = self.shape {
            parts.push(
                match shape {
                    None => l.pick("实心盒", "Square"),
                    Some(BrushShape::Ellipsoid) => l.pick("椭球", "Round"),
                    Some(BrushShape::Diamond) => l.pick("菱形", "Diamond"),
                    Some(BrushShape::CylinderY) => l.pick("圆柱", "Cylinder"),
                    Some(BrushShape::ParabolicDome) => l.pick("抛物面圆顶", "Parabolic dome"),
                    Some(BrushShape::HollowBox) => l.pick("空心盒", "Hollow box"),
                    Some(BrushShape::EllipsoidShell) => l.pick("椭球壳", "Ellipsoid shell"),
                    Some(BrushShape::CylinderShellY) => l.pick("圆柱壳", "Cylinder shell"),
                }
                .to_owned(),
            );
        }
        if let Some([x, y, z]) = self.size {
            parts.push(format!("{x}×{y}×{z}"));
        }
        parts.join(" · ")
    }
}
pub(crate) struct PreparedImport {
    settings: Settings,
    value: serde_json::Value,
}
impl PreparedImport {
    pub(crate) fn value(&self) -> &serde_json::Value {
        &self.value
    }
}
pub(crate) fn prepare_legacy(app: &App, legacy: &LegacySettings) -> Result<PreparedImport, String> {
    let mut saved = Saved::capture(app);
    if let Some(mode) = legacy.mode {
        saved.settings.mode = mode;
    }
    if let Some(shape) = legacy.shape {
        saved.settings.shape = shape;
    }
    if let Some([x, y, z]) = legacy.size {
        saved.settings.size = format!("{x},{y},{z}");
    }
    saved.validate()?;
    Ok(PreparedImport {
        value: serde_json::to_value(&saved).map_err(|error| error.to_string())?,
        settings: saved.settings,
    })
}
pub(crate) fn install_legacy(app: &mut App, prepared: PreparedImport) {
    app.block_edit.cursor.clear();
    app.block_edit.stroke = prepared.settings;
    app.block_edit.brush_preferences_report =
        "旧笔刷参数已合并并加入保存队列；目标和替换规则保持当前值。".into();
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    settings: Settings,
    target: String,
    biome: String,
}
impl Saved {
    fn capture(app: &App) -> Self {
        Self {
            version: 1,
            settings: app.block_edit.stroke.clone(),
            target: app.edit_target.clone(),
            biome: app.block_edit.biome_target.clone(),
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("不支持笔刷设置版本 {}", self.version));
        }
        let config = Config {
            input_epoch: 0,
            budget_mib: "256".into(),
            visual: None,
            mode: self.settings.mode,
            size: self.settings.size.clone(),
            offset: self.settings.offset.clone(),
            shape: self.settings.shape,
            hollow: self.settings.hollow,
            rules: self.settings.rules.clone(),
            target: self.target.clone(),
        };
        config.dimensions()?;
        self.settings
            .offset
            .trim()
            .parse::<i32>()
            .map_err(|_| "笔刷偏移必须为整数")?;
        parse_state(&self.target).map_err(|e| e.to_string())?;
        // 模式本身是有效偏好；空群系目标 / 空替换表仍可在面板中配置。
        // 真正执行时分别由 BiomeTarget::parse / Config::paint 拒绝未完成的操作。
        if !self.biome.is_empty() {
            BiomeTarget::parse(&self.biome).map_err(|e| e.to_string())?;
        }
        if !self.settings.rules.is_empty() {
            Config {
                mode: Mode::Replace,
                ..config
            }
            .paint()?;
        }
        Ok(())
    }
    fn from_values(
        values: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<Self>, String> {
        values
            .get("brush")
            .map(|value| {
                let result: Self = serde_json::from_value(value.clone())
                    .map_err(|e| format!("笔刷设置无效：{e}"))?;
                result.validate()?;
                Ok(result)
            })
            .transpose()
    }
}
pub(crate) fn restore(app: &mut App) {
    let result = launch::read_rust_prefs()
        .map_err(|e| e.to_string())
        .and_then(|values| Saved::from_values(&values));
    match result {
        Ok(Some(saved)) => {
            app.block_edit.stroke = saved.settings;
            app.edit_target = saved.target;
            app.block_edit.biome_target = saved.biome;
        }
        Ok(None) => {}
        Err(error) => {
            app.view_state.optional::<()>("brush", Err(error.clone()));
            app.block_edit.brush_preferences_report = format!("笔刷设置读取失败：{error}");
            app.messages
                .push(app.block_edit.brush_preferences_report.clone());
        }
    }
}
pub(crate) fn remember(app: &mut App) {
    let result = (|| {
        let saved = Saved::capture(app);
        saved.validate()?;
        app.view_state
            .queue_brush(serde_json::to_value(saved).map_err(|e| e.to_string())?)
    })();
    app.block_edit.brush_preferences_report = match result {
        Ok(()) => "笔刷参数已加入自动保存队列。".into(),
        Err(error) => {
            log::error!("[brush-preferences] {error}");
            format!("当前参数未保存：{error}")
        }
    };
}

#[cfg(test)]
pub(super) fn run_tests() {
    // Preferences change process-wide environment; run this case in its own
    // harness process so other independently named tests remain parallel-safe.
    if std::env::var_os("MCEDIT2_PREFERENCES_TEST_CHILD").is_none() {
        let name = std::thread::current()
            .name()
            .expect("test harness names its threads")
            .to_owned();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--test-threads=1"])
            .env("MCEDIT2_PREFERENCES_TEST_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success(), "isolated preferences test failed");
        return;
    }
    use std::{ffi::OsString, path::PathBuf};
    struct Scope {
        root: PathBuf,
        previous: Option<OsString>,
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var("XDG_CONFIG_HOME", previous);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
            let actual = self.root.canonicalize().unwrap();
            assert_eq!(
                actual.parent(),
                Some(std::env::temp_dir().canonicalize().unwrap().as_path())
            );
            assert!(actual
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("mcedit2-brush-preferences-"));
            std::fs::remove_dir_all(actual).unwrap();
        }
    }
    let root = std::env::temp_dir().join(format!(
        "mcedit2-brush-preferences-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let scope = Scope {
        root,
        previous: std::env::var_os("XDG_CONFIG_HOME"),
    };
    std::env::set_var("XDG_CONFIG_HOME", &scope.root);
    launch::update_rust_prefs(|values| {
        values.insert("unrelated".into(), serde_json::json!({"保留":42}));
        values.insert(
            "views".into(),
            serde_json::json!({"version":1,"entries":[],"recent":[]}),
        );
        Ok(())
    })
    .unwrap();
    let mut app = App::default();
    app.view_state = crate::view_state::State::load().unwrap().0;
    app.edit_target = "minecraft:gold_block".into();
    app.block_edit.biome_target = "custom:alpine".into();
    app.block_edit.stroke = Settings {
        enabled: true,
        mode: Mode::Replace,
        size: "3,3,3".into(),
        offset: "-2".into(),
        shape: Some(BrushShape::Ellipsoid),
        hollow: true,
        rules: vec![
            Rule {
                sources: "minecraft:stone | minecraft:oak_log[axis=y]".into(),
                target: "minecraft:dirt".into(),
            },
            Rule {
                sources: "minecraft:stone".into(),
                target: "minecraft:gold_block".into(),
            },
        ],
    };
    remember(&mut app);
    assert!(app.block_edit.brush_preferences_report.contains("队列"));
    app.block_edit.stroke.size = "4,3,3".into();
    remember(&mut app);
    let mut expected = Saved::capture(&app);
    expected.settings.enabled = false;
    drop(app); // 关闭时排空现有写入队列，验证最后一组设置而不是仅验证序列化。
    let values = launch::read_rust_prefs().unwrap();
    assert_eq!(values["unrelated"], serde_json::json!({"保留":42}));
    assert_eq!(
        values["views"],
        serde_json::json!({"version":1,"entries":[],"recent":[]})
    );
    assert_eq!(Saved::from_values(&values).unwrap().unwrap(), expected);
    let mut app = App::default();
    restore(&mut app);
    assert!(!app.block_edit.stroke.enabled);
    assert_eq!(app.block_edit.stroke, expected.settings);
    assert_eq!(app.edit_target, expected.target);
    assert_eq!(app.block_edit.biome_target, expected.biome);
    crate::selection_preferences::run_tests();
    crate::desktop_state::run_tests();
    crate::nbt_search::run_tests();
    let mut valid = expected.clone();
    valid.settings.mode = Mode::Fill;
    valid.settings.rules.clear();
    valid.settings.hollow = false;
    valid.settings.offset = "0".into();
    valid.validate().unwrap();
    let mut invalid = values.clone();
    invalid.get_mut("brush").unwrap()["version"] = serde_json::json!(2);
    assert!(Saved::from_values(&invalid).is_err());
    invalid = values.clone();
    invalid.get_mut("brush").unwrap()["settings"]["shape"] = serde_json::json!("未知形状");
    assert!(Saved::from_values(&invalid).is_err());
    app.view_state = crate::view_state::State::load().unwrap().0;
    app.block_edit.stroke.size = "未完成输入".into();
    remember(&mut app);
    assert!(app.block_edit.brush_preferences_report.contains("未保存"));
    drop(app);
    assert_eq!(
        Saved::from_values(&launch::read_rust_prefs().unwrap())
            .unwrap()
            .unwrap(),
        expected
    );
    launch::update_rust_prefs(|values| {
        values.get_mut("brush").unwrap()["version"] = serde_json::json!(2);
        Ok(())
    })
    .unwrap();
    let mut app = App::default();
    restore(&mut app);
    assert!(!app.messages.is_empty());
    assert!(app.block_edit.brush_preferences_report.contains("读取失败"));
    // 同批删除 / 输入仍携带旧列表身份，不能修改移到原索引的其他规则。
    app.block_edit.stroke.rules = expected.settings.rules.clone();
    let revision = app.block_edit.brush_rule_revision;
    let remaining = app.block_edit.stroke.rules[1].clone();
    let _ = update(&mut app, Action::RemoveRule(revision, 0));
    let _ = update(
        &mut app,
        Action::RuleTarget(revision, 0, "minecraft:air".into()),
    );
    let _ = update(&mut app, Action::RemoveRule(revision, 0));
    assert_eq!(app.block_edit.stroke.rules, vec![remaining]);
    assert!(app.edit_report.contains("列表已改变"));
    let revision = app.block_edit.brush_rule_revision;
    let _ = update(
        &mut app,
        Action::RuleTarget(revision, 0, "minecraft:air".into()),
    );
    assert_eq!(app.block_edit.stroke.rules[0].target, "minecraft:air");
    // A damaged optional section must neither prevent editor startup nor be
    // overwritten by automatic preference persistence.
    launch::update_rust_prefs(|values| {
        values.insert("desktop".into(), serde_json::json!({"version":99}));
        values.insert("selection".into(), serde_json::json!({"version":99}));
        Ok(())
    })
    .unwrap();
    let path = launch::rust_prefs_path().unwrap();
    let original = std::fs::read(&path).unwrap();
    let (mut booted, task) = App::boot(Default::default(), true).unwrap();
    assert!(booted
        .messages
        .iter()
        .any(|message| message.contains("desktop")));
    assert!(booted
        .block_edit
        .brush_preferences_report
        .contains("读取失败"));
    assert!(booted
        .selection_edit
        .preferences_report
        .contains("读取失败"));
    remember(&mut booted);
    crate::selection_preferences::remember(&mut booted);
    crate::desktop_state::remember(&mut booted);
    drop(task);
    drop(booted);
    assert_eq!(std::fs::read(path).unwrap(), original);
}
