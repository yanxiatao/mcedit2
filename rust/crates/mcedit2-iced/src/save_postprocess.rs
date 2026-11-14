//! 保存预览与确认共用派生数据合同；切换策略必须重新预检，旧版重算产物携带完整邻域证明。
use crate::{NativeRegionCandidate, Session, WorldSummary};
use iced::{
    widget::{checkbox, column, text, text_input},
    Element,
};
use mcedit2_world::{
    chunk_postprocess::Policy,
    legacy_lighting::{Dimension, Traits},
    mixed_transaction::MixedCandidate,
};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    enabled: bool,
    skylight: bool,
    traits_file: String,
}
impl Settings {
    fn new(dimension: &str) -> Self {
        Self {
            enabled: false,
            skylight: dimension == "minecraft:overworld",
            traits_file: String::new(),
        }
    }
}
/// 按真实世界路径和维度保存显式参数；不保存属性表内容或冻结的世界合同。
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    worlds: std::collections::BTreeMap<
        std::path::PathBuf,
        std::collections::BTreeMap<String, Settings>,
    >,
}
impl Saved {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (root, dimensions) in &self.worlds {
            if !root.is_absolute() || dimensions.keys().any(|name| name.trim().is_empty()) {
                return Err("保存光照设置的存档路径或维度无效".into());
            }
        }
        Ok(())
    }
}
#[derive(Default)]
pub(crate) struct Panel {
    pub(crate) saved: Saved,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Source {
    root: std::path::PathBuf,
    dimension: String,
}
impl Source {
    fn current(app: &crate::App) -> Option<Self> {
        if app.structures.active.is_some() || app.desktop.show_recent {
            return None;
        }
        let (wi, di) = app.current_key()?;
        let world = app.worlds.get(wi)?;
        Some(Self {
            root: world.root.clone(),
            dimension: world.dimensions.get(di)?.name.clone(),
        })
    }
    fn message(&self, change: Change) -> crate::Message {
        crate::Message::SavePostprocess(Action::new(&self.root, &self.dimension, change))
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Change {
    Enabled(bool),
    Skylight(bool),
    TraitsFile(String),
    Browse,
    Reset,
}
#[derive(Debug, Clone)]
pub(crate) struct Action {
    source: Source,
    change: Change,
}
impl Action {
    pub(crate) fn new(root: &std::path::Path, dimension: &str, change: Change) -> Self {
        Self {
            source: Source {
                root: root.to_path_buf(),
                dimension: dimension.into(),
            },
            change,
        }
    }
}
/// 冻结发起预检时的配置，文件与世界合同只在后台实际读取。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Request {
    source: Source,
    settings: Option<Settings>,
}
impl Request {
    pub(crate) fn policy(&self, world: &WorldSummary) -> Result<Policy, String> {
        if self.source.root != world.root {
            return Err("保存策略请求属于另一存档".into());
        }
        let dimension = self.source.dimension.as_str();
        let Some(settings) = self.settings.as_ref().filter(|settings| settings.enabled) else {
            return Ok(Policy::NativeInvalidation);
        };
        let profile = Dimension::confirm(
            world,
            dimension,
            settings.skylight,
            "用户确认的旧数字格式维度光照合同",
        )
        .map_err(|error| error.to_string())?;
        let traits = if settings.traits_file.trim().is_empty() {
            if world.data_version != Some(1343) {
                return Err("当前来源没有已绑定的原版光照属性表，请提供实际世界的数字 ID 光照属性 JSON；不按 1.12.2 推测其他版本。".into());
            }
            Traits::vanilla_1_12(true).map_err(|error| error.to_string())?
        } else {
            use std::io::Read;
            let path = std::path::Path::new(settings.traits_file.trim());
            let meta = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
                return Err("光照属性必须为不超过 1 MiB 的普通 JSON 文件".into());
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|error| error.to_string())?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() > 1024 * 1024 {
                return Err("读取时光照属性文件超过预算".into());
            }
            serde_json::from_slice(&bytes).map_err(|error| format!("光照属性文件无效：{error}"))?
        };
        Ok(Policy::LegacyRelight {
            profile,
            traits,
            operation_budget: 50_000_000,
            memory_budget: 256 * 1024 * 1024,
        })
    }
}
impl Panel {
    fn settings(&self, root: &std::path::Path, dimension: &str) -> Option<&Settings> {
        self.saved.worlds.get(root)?.get(dimension)
    }
    pub(crate) fn request(&self, world: &WorldSummary, dimension: &str) -> Request {
        Request {
            source: Source {
                root: world.root.clone(),
                dimension: dimension.into(),
            },
            settings: self.settings(&world.root, dimension).cloned(),
        }
    }
    pub(crate) fn view<'a>(
        &'a self,
        root: &std::path::Path,
        dimension: &str,
    ) -> Element<'a, crate::Message> {
        let source = Source {
            root: root.to_path_buf(),
            dimension: dimension.into(),
        };
        let settings = self.settings(root, dimension);
        let enabled = settings.is_some_and(|settings| settings.enabled);
        let skylight = settings.map_or(dimension == "minecraft:overworld", |settings| {
            settings.skylight
        });
        let path = settings.map_or("", |settings| settings.traits_file.as_str());
        let enabled_source = source.clone();
        let mut body = column![
            text(format!("旧版方块保存光照 · {dimension}")).size(16),
            checkbox(enabled)
                .label("确认重算旧数字格式光照；非 1.12.2 或自定义 ID 须提供实际属性文件")
                .on_toggle(move |value| enabled_source.message(Change::Enabled(value))),
        ]
        .spacing(5);
        if enabled {
            let sky_source = source.clone();
            let path_source = source.clone();
            body = body.push(checkbox(skylight).label("当前维度具有天空光（请核对模组维度）")
                .on_toggle(move |value| sky_source.message(Change::Skylight(value))))
                .push(iced::widget::row![
                    text_input("实际光照属性 JSON；仅 1.12.2 可留空使用内置定义", path)
                        .on_input(move |value| path_source.message(Change::TraitsFile(value))),
                    iced::widget::button("浏览…").on_press(source.message(Change::Browse)),
                ].spacing(6))
                .push(text("已有完整光照时增量更新；未完成时按实际方块完整重算高度和光照。来源不足、未知 ID、取消或预算不足会报告错误，不写入部分结果。").size(12));
        }
        if settings.is_some() {
            body = body.push(
                iced::widget::button("重设本维度的保存光照设置")
                    .on_press(source.message(Change::Reset)),
            );
        }
        body.into()
    }
}
/// 文件浏览绑定同一世界 / 维度的完整参数草稿，重设或改参数后不套用旧选择。
pub(crate) fn selection(app: &crate::App) -> Option<Request> {
    let source = Source::current(app)?;
    let settings = app
        .save_postprocess
        .settings(&source.root, &source.dimension)
        .cloned();
    Some(Request { source, settings })
}
pub(crate) fn traits_path(app: &crate::App) -> Option<&str> {
    let source = Source::current(app)?;
    Some(
        app.save_postprocess
            .settings(&source.root, &source.dimension)
            .map_or("", |settings| settings.traits_file.as_str()),
    )
}
pub(crate) fn chosen(app: &mut crate::App, path: String) -> iced::Task<crate::Message> {
    let source = Source::current(app).expect("属性表选择结果已核对活动世界归属");
    update(
        app,
        Action {
            source,
            change: Change::TraitsFile(path),
        },
    )
}
pub(crate) fn update(app: &mut crate::App, action: Action) -> iced::Task<crate::Message> {
    if Source::current(app).as_ref() != Some(&action.source) {
        let report = "保存光照设置属于另一存档或维度，未应用旧操作";
        log::info!("[save] {report}");
        app.messages.push(report.into());
        return iced::Task::none();
    }
    if matches!(action.change, Change::Browse) {
        return crate::dialogs::advanced_file(app, crate::AdvancedFile::LightTraits, false);
    }
    let Source { root, dimension } = action.source;
    if matches!(action.change, Change::Reset) {
        if let Some(dimensions) = app.save_postprocess.saved.worlds.get_mut(&root) {
            dimensions.remove(&dimension);
            if dimensions.is_empty() {
                app.save_postprocess.saved.worlds.remove(&root);
            }
        }
    } else {
        let settings = app
            .save_postprocess
            .saved
            .worlds
            .entry(root)
            .or_default()
            .entry(dimension.clone())
            .or_insert_with(|| Settings::new(&dimension));
        match action.change {
            Change::Enabled(value) => settings.enabled = value,
            Change::Skylight(value) => settings.skylight = value,
            Change::TraitsFile(value) => settings.traits_file = value,
            Change::Browse | Change::Reset => unreachable!("浏览和重设已单独处理"),
        }
    }
    app.save_cancelled
        .store(true, std::sync::atomic::Ordering::Release);
    app.save_planning = false;
    app.save_preview = None;
    app.save_phrase.clear();
    app.be_save_phrase.clear();
    iced::Task::none()
}
#[derive(Debug, Clone)]
enum Kind {
    Native(Vec<NativeRegionCandidate>),
    Mixed(Vec<MixedCandidate>),
}
#[derive(Debug, Clone)]
pub(crate) struct Candidates {
    kind: Kind,
    revision: std::sync::Arc<()>,
    preview: mcedit2_world::mixed_transaction::TerrainPreview,
}
impl Candidates {
    /// 确认阶段按冻结的模式重新规划；模式不随新候选静默变化。
    pub(crate) fn prepare(
        session: &Session,
        policy: &Policy,
        be: bool,
        timestamp: u32,
    ) -> Result<Self, String> {
        let plans = Self::prepare_auto_cancellable(session, policy, timestamp, || false)?;
        if plans.requires_nbt_confirmation() != be {
            return Err("保存确认方式与当前候选内容不一致，请重新预检".into());
        }
        Ok(plans)
    }
    /// 普通保存也预览完整会话，实际实体 / BE 证明决定是否要求 NBT 确认。
    pub(crate) fn prepare_auto_cancellable(
        session: &Session,
        policy: &Policy,
        timestamp: u32,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, String> {
        if cancelled() {
            return Err("保存预检已取消".into());
        }
        let all = session
            .prepare_postprocessed_candidates(policy, timestamp, &mut cancelled)
            .map_err(|error| error.to_string())?;
        if cancelled() {
            return Err("保存预检已取消".into());
        }
        let preview = mcedit2_world::mixed_transaction::terrain_preview(&all)
            .map_err(|error| error.to_string())?;
        let native = all.iter().all(|candidate| {
            matches!(candidate,
            MixedCandidate::Region {dimension, candidate} if dimension == session.dimension() && candidate.section_proofs.is_empty())
        });
        let kind = if native {
            Kind::Native(
                all.into_iter()
                    .map(|candidate| match candidate {
                        MixedCandidate::Region { candidate, .. } => candidate,
                        _ => unreachable!("已核对全部候选为当前维度 Region"),
                    })
                    .collect(),
            )
        } else {
            Kind::Mixed(all)
        };
        Ok(Self {
            kind,
            preview,
            revision: session.revision(),
        })
    }
    pub(crate) fn native(&self) -> Option<&[NativeRegionCandidate]> {
        match &self.kind {
            Kind::Native(plans) => Some(plans),
            Kind::Mixed(_) => None,
        }
    }
    /// 混合保存接收刚刚比较通过的同一候选，避免再按另一份属性表规划一次。
    pub(crate) fn into_mixed(self, dimension: &str) -> Vec<MixedCandidate> {
        match self.kind {
            Kind::Mixed(plans) => plans,
            Kind::Native(plans) => plans
                .into_iter()
                .map(|candidate| MixedCandidate::Region {
                    dimension: dimension.into(),
                    candidate,
                })
                .collect(),
        }
    }
    pub(crate) fn requires_nbt_confirmation(&self) -> bool {
        self.preview.requires_nbt_confirmation
    }
    pub(crate) fn stats(&self) -> (usize, usize, usize) {
        (
            self.preview.file_count,
            self.preview.chunk_count,
            self.preview.bytes,
        )
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.revision, &other.revision)
            && self.preview.fingerprint == other.preview.fingerprint
    }
    pub(crate) fn summaries(&self) -> Vec<String> {
        self.preview.summaries.clone()
    }
}
