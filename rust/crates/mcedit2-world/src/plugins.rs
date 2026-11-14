//! Versioned, declarative safe-plugin foundation for the Rust rewrite.
//! No arbitrary executable, Python, DLL, Wasm, file/network or shell access.
//! The first safe operation delegates to the existing original NBT checked
//! and Undo-able Region edit preview: one bounded conditional replacement.
use crate::editing::{
    parse_state, Bounds, BrushShape, EditWorkBudget, EditWorkProgress, Preview, Session,
};
use crate::WorldError;
use serde::Deserialize;
use std::fs;
use std::path::Path;

fn refuse(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub schema_version: u32,
    pub name: String,
    pub description: Option<String>,
    pub only_data_version: Option<i32>,
    pub only_dimension: Option<String>,
    pub operation: PluginOperation,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginOperation {
    ReplaceBlocks {
        minimum: [i32; 3],
        maximum: [i32; 3],
        with_state: String,
        only_if_state: Option<String>,
    },
    Brush {
        minimum: [i32; 3],
        maximum: [i32; 3],
        shape: PluginBrush,
        with_state: String,
        only_if_state: Option<String>,
    },
}
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginBrush {
    HollowBox,
    Ellipsoid,
    EllipsoidShell,
    CylinderY,
    CylinderShellY,
}
impl PluginBrush {
    pub fn shape(self) -> BrushShape {
        match self {
            Self::HollowBox => BrushShape::HollowBox,
            Self::Ellipsoid => BrushShape::Ellipsoid,
            Self::EllipsoidShell => BrushShape::EllipsoidShell,
            Self::CylinderY => BrushShape::CylinderY,
            Self::CylinderShellY => BrushShape::CylinderShellY,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifestV2 {
    pub schema_version: u32,
    pub name: String,
    pub description: Option<String>,
    pub only_data_version: Option<i32>,
    pub only_dimension: Option<String>,
    pub operations: Vec<PluginOperation>,
}
pub fn load(path: &Path) -> Result<PluginManifest, WorldError> {
    if !path
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.ends_with(".mcedit2plugin.json"))
    {
        return Err(refuse("Only .mcedit2plugin.json manifests are accepted"));
    }
    let info = fs::symlink_metadata(path)?;
    if !info.file_type().is_file() || info.file_type().is_symlink() || info.len() > 32 * 1024 {
        return Err(refuse(
            "Plugin manifest must be a small regular file <=32 KiB",
        ));
    }
    let manifest: PluginManifest = serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| refuse(format!("Invalid declarative plugin JSON: {e}")))?;
    if manifest.schema_version != 1
        || manifest.name.is_empty()
        || manifest.name.len() > 96
        || manifest.name.contains('\0')
        || manifest
            .description
            .as_ref()
            .is_some_and(|v| v.len() > 1024)
    {
        return Err(refuse("Unsupported plugin version or invalid metadata"));
    }
    Ok(manifest)
}
impl PluginManifest {
    pub fn prepare(&self, session: &Session) -> Result<Preview, WorldError> {
        self.prepare_with_budget(session, EditWorkBudget::default(), || false, |_| {})
    }
    pub fn prepare_with_budget(
        &self,
        session: &Session,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        if self.schema_version != 1 || !valid_meta(&self.name, self.description.as_deref()) {
            return Err(refuse("声明式插件版本或元数据无效"));
        }
        if self
            .only_data_version
            .is_some_and(|version| session.original_world().data_version != Some(version))
        {
            return Err(refuse("Plugin requires an exact original DataVersion"));
        }
        if self
            .only_dimension
            .as_ref()
            .is_some_and(|dimension| dimension != session.dimension())
        {
            return Err(refuse("Plugin dimension does not match the original world"));
        }
        preview_op_with_budget(session, &self.operation, budget, cancelled, progress)
    }
}
pub(crate) fn preview_op_with_budget(
    session: &Session,
    operation: &PluginOperation,
    budget: EditWorkBudget,
    cancelled: impl FnMut() -> bool,
    progress: impl FnMut(EditWorkProgress),
) -> Result<Preview, WorldError> {
    let (minimum, maximum, state, filter, shape) = match operation {
        PluginOperation::ReplaceBlocks {
            minimum,
            maximum,
            with_state,
            only_if_state,
        } => (*minimum, *maximum, with_state, only_if_state, None),
        PluginOperation::Brush {
            minimum,
            maximum,
            shape,
            with_state,
            only_if_state,
        } => (
            *minimum,
            *maximum,
            with_state,
            only_if_state,
            Some(shape.shape()),
        ),
    };
    let bounds = Bounds::spatial(minimum.into(), maximum.into())?;
    let target = parse_state(state)?;
    let filter = filter
        .as_ref()
        .map(|state| parse_state(state))
        .transpose()?;
    session.prepare_box_cancellable(
        bounds,
        shape,
        &target,
        filter.as_ref(),
        budget,
        cancelled,
        progress,
    )
}
fn manifest_bytes(path: &Path) -> Result<Vec<u8>, WorldError> {
    if !path
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.ends_with(".mcedit2plugin.json"))
    {
        return Err(refuse("Only .mcedit2plugin.json manifests accepted"));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > 32 * 1024 {
        return Err(refuse("Plugin must be a regular file of <=32 KiB"));
    }
    Ok(fs::read(path)?)
}
fn valid_meta(name: &str, description: Option<&str>) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && !name.contains('\0')
        && description.is_none_or(|s| s.len() <= 1024)
}
impl PluginManifestV2 {
    pub fn prepare(&self, session: &Session) -> Result<Preview, WorldError> {
        self.prepare_with_budget(session, EditWorkBudget::default(), || false, |_| {})
    }
    pub fn prepare_with_budget(
        &self,
        session: &Session,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        if self.schema_version != 2
            || !valid_meta(&self.name, self.description.as_deref())
            || self.operations.is_empty()
        {
            return Err(refuse(
                "Invalid version-2 plugin metadata or operation count",
            ));
        }
        if self
            .only_data_version
            .is_some_and(|dv| session.original_world().data_version != Some(dv))
        {
            return Err(refuse("Plugin requires exact source DataVersion"));
        }
        if self
            .only_dimension
            .as_deref()
            .is_some_and(|d| d != session.dimension())
        {
            return Err(refuse("Plugin requires matching original dimension"));
        }
        session.prepare_plugin_sequence_with_budget(&self.operations, budget, cancelled, progress)
    }
}
pub fn prepare_file(session: &Session, path: &Path) -> Result<Preview, WorldError> {
    prepare_file_with_budget(session, path, EditWorkBudget::default(), || false, |_| {})
}
/// 独立结构使用本文件的局部坐标与真实版本约束，整组操作形成一条文档历史。
pub fn prepare_document_file(
    document: &crate::structure_document::Document,
    path: &Path,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<crate::structure_document::PreparedScript, WorldError> {
    let bytes = manifest_bytes(path)?;
    let base: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| refuse(e.to_string()))?;
    let (name, description, version, dimension, operations) =
        match base.get("schema_version").and_then(|value| value.as_u64()) {
            Some(1) => {
                let value: PluginManifest =
                    serde_json::from_slice(&bytes).map_err(|e| refuse(e.to_string()))?;
                (
                    value.name,
                    value.description,
                    value.only_data_version,
                    value.only_dimension,
                    vec![value.operation],
                )
            }
            Some(2) => {
                let value: PluginManifestV2 =
                    serde_json::from_slice(&bytes).map_err(|e| refuse(e.to_string()))?;
                (
                    value.name,
                    value.description,
                    value.only_data_version,
                    value.only_dimension,
                    value.operations,
                )
            }
            _ => return Err(refuse("不支持的声明式插件版本")),
        };
    if !valid_meta(&name, description.as_deref()) || operations.is_empty() {
        return Err(refuse("声明式插件元数据或操作数量无效"));
    }
    let source = document.content();
    if version.is_some_and(|version| Some(version) != source.data_version) {
        return Err(refuse("声明式插件要求准确来源版本"));
    }
    if dimension
        .as_deref()
        .is_some_and(|dimension| source.entity_dimension.as_deref() != Some(dimension))
    {
        return Err(refuse("独立文件没有声明插件所需的真实维度来源"));
    }
    let area = crate::editing::SelectionRegion::from(Bounds {
        minimum: [0; 3],
        maximum: source.size.map(|v| i32::from(v) - 1),
    });
    let snapshot = document.capture_script_cancellable(area, false, budget, &mut cancelled)?;
    let mut blocks = snapshot.clipboard().blocks.clone();
    for operation in operations {
        let (minimum, maximum, state, filter, shape) = match operation {
            PluginOperation::ReplaceBlocks {
                minimum,
                maximum,
                with_state,
                only_if_state,
            } => (minimum, maximum, with_state, only_if_state, None),
            PluginOperation::Brush {
                minimum,
                maximum,
                with_state,
                only_if_state,
                shape,
            } => (
                minimum,
                maximum,
                with_state,
                only_if_state,
                Some(shape.shape()),
            ),
        };
        if (0..3).any(|axis| {
            minimum[axis] < 0
                || maximum[axis] < minimum[axis]
                || maximum[axis] >= i32::from(source.size[axis])
        }) {
            return Err(refuse("声明式插件操作超出结构局部范围"));
        }
        let bounds = Bounds { minimum, maximum };
        let state = parse_state(&state)?;
        let filter = filter.as_deref().map(parse_state).transpose()?;
        for y in minimum[1]..=maximum[1] {
            for z in minimum[2]..=maximum[2] {
                for x in minimum[0]..=maximum[0] {
                    let index = (y as usize * usize::from(source.size[2]) + z as usize)
                        * usize::from(source.size[0])
                        + x as usize;
                    if index.is_multiple_of(4096) && cancelled() {
                        return Err(refuse("声明式结构插件已取消"));
                    }
                    if source.mask.as_ref().is_some_and(|mask| !mask[index])
                        || shape.is_some_and(|shape| {
                            !crate::editing::brush_inside(shape, bounds, (x, y, z))
                        })
                    {
                        continue;
                    }
                    if filter
                        .as_ref()
                        .is_none_or(|filter| blocks[index] == *filter)
                    {
                        blocks[index] = state.clone();
                    }
                }
            }
        }
    }
    let content = crate::editing::snapshot::Content::capture(snapshot.clipboard())?;
    document.prepare_script_difference_cancellable(&snapshot, &blocks, &content, budget, cancelled)
}
pub fn prepare_file_with_budget(
    session: &Session,
    path: &Path,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
    mut progress: impl FnMut(EditWorkProgress),
) -> Result<Preview, WorldError> {
    if cancelled() {
        return Err(refuse("声明式插件读取已取消"));
    }
    let bytes = manifest_bytes(path)?;
    let base: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| refuse(format!("Invalid plugin JSON: {e}")))?;
    match base.get("schema_version").and_then(|s| s.as_u64()) {
        Some(1) => load(path)?.prepare_with_budget(session, budget, &mut cancelled, &mut progress),
        Some(2) => {
            let manifest: PluginManifestV2 = serde_json::from_slice(&bytes)
                .map_err(|e| refuse(format!("Invalid v2 plugin schema: {e}")))?;
            manifest.prepare_with_budget(session, budget, &mut cancelled, &mut progress)
        }
        _ => Err(refuse("Unsupported declarative plugin schema version")),
    }
}
