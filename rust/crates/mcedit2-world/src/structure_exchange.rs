//! 各结构文件的统一入口；格式显式选择，载荷无法表达时保留编解码器的错误。
use crate::{
    clipboard::{self, BlockClipboard},
    editing::EditWorkBudget,
    schematic, WorldError,
};
use std::{fmt, path::Path};
#[path = "structure_export.rs"]
mod export;
pub use export::{ExportOptions, ExportSummary};

/// 格式同时决定扩展名与读写器，不以失败后重试其他格式来猜测内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    #[default]
    Clipboard,
    Legacy,
    SpongeV2,
    SpongeV3,
    Structure,
}
impl Format {
    /// 由明确扩展名和格式版本识别导入器，不在解码失败后尝试其他格式。
    /// 普通 NBT 返回 None，留给独立文档入口处理。
    pub fn detect(path: &Path) -> Result<Option<Self>, WorldError> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "mcedit2clip" => Ok(Some(Self::Clipboard)),
            "schematic" => Ok(Some(Self::Legacy)),
            "schem" | "nbt" => {
                let bytes = crate::nbt_file::read_nbt(path)?;
                let root = crate::sections::parse_root(&bytes)
                    .map_err(|error| WorldError::Invalid(error.to_string()))?;
                if extension == "nbt" {
                    return Ok((root.field("size").is_some()
                        && (root.field("palette").is_some() || root.field("palettes").is_some())
                        && root.field("blocks").is_some())
                    .then_some(Self::Structure));
                }
                let container = root.field("Schematic").unwrap_or(&root);
                let version = container
                    .require("Version", 3)
                    .and_then(crate::sections::Tag::i32)
                    .map_err(|error| WorldError::Invalid(error.to_string()))?;
                match (version, root.field("Schematic").is_some()) {
                    (2, false) => Ok(Some(Self::SpongeV2)),
                    (3, true) => Ok(Some(Self::SpongeV3)),
                    _ => Err(WorldError::Invalid(format!(
                        "未知 Sponge 结构版本或根布局：{version}"
                    ))),
                }
            }
            _ => Ok(None),
        }
    }
    pub const ALL: [Self; 5] = [
        Self::Clipboard,
        Self::Legacy,
        Self::SpongeV2,
        Self::SpongeV3,
        Self::Structure,
    ];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Clipboard => "mcedit2clip",
            Self::Legacy => "schematic",
            Self::SpongeV2 | Self::SpongeV3 => "schem",
            Self::Structure => "nbt",
        }
    }
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Clipboard => &["mcedit2clip"],
            Self::Legacy => &["schematic"],
            Self::SpongeV2 | Self::SpongeV3 => &["schem"],
            Self::Structure => &["nbt"],
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Clipboard => "MCEdit2 剪贴板",
            Self::Legacy => "旧式 MCEdit 结构",
            Self::SpongeV2 => "Sponge v2",
            Self::SpongeV3 => "Sponge v3",
            Self::Structure => "原版结构 NBT",
        }
    }
    pub fn read(
        self,
        world: &impl crate::structure_context::CodecContext,
        dimension: &str,
        path: &Path,
    ) -> Result<BlockClipboard, WorldError> {
        self.read_with_budget(world, dimension, path, EditWorkBudget::default())
    }
    pub fn read_with_budget(
        self,
        world: &impl crate::structure_context::CodecContext,
        dimension: &str,
        path: &Path,
        budget: EditWorkBudget,
    ) -> Result<BlockClipboard, WorldError> {
        match self {
            Self::Clipboard => clipboard::load_with_budget(world, dimension, path, budget),
            Self::Legacy => schematic::import_legacy_with_budget(world, dimension, path, budget),
            Self::SpongeV2 => {
                schematic::import_sponge_v2_with_budget(world, dimension, path, budget)
            }
            Self::SpongeV3 => {
                schematic::import_sponge_v3_with_budget(world, dimension, path, budget)
            }
            Self::Structure => {
                schematic::import_structure_with_budget(world, dimension, path, budget)
            }
        }
    }
    /// 对保存候选和恢复日志中的原始字节使用同一完整编解码合同。
    pub fn decode_with_budget(
        self,
        context: &impl crate::structure_context::CodecContext,
        dimension: &str,
        bytes: &[u8],
        budget: EditWorkBudget,
    ) -> Result<BlockClipboard, WorldError> {
        match self {
            Self::Clipboard => clipboard::decode_with_budget(context, dimension, bytes, budget),
            Self::Legacy => schematic::decode_legacy_with_budget(context, dimension, bytes, budget),
            Self::SpongeV2 => {
                schematic::decode_sponge_v2_with_budget(context, dimension, bytes, budget)
            }
            Self::SpongeV3 => {
                schematic::decode_sponge_v3_with_budget(context, dimension, bytes, budget)
            }
            Self::Structure => {
                schematic::decode_structure_with_budget(context, dimension, bytes, budget)
            }
        }
    }
    /// 只编码，不创建文件；来源校验由持有真实文件身份的保存或导出入口承担。
    pub fn encode_with_budget(
        self,
        context: &impl crate::structure_context::CodecContext,
        clip: &BlockClipboard,
        budget: EditWorkBudget,
    ) -> Result<Vec<u8>, WorldError> {
        match self {
            Self::Clipboard => clipboard::encode_with_budget(context, clip, budget),
            Self::Legacy => schematic::encode_legacy_with_budget(context, clip, budget),
            Self::SpongeV2 => schematic::encode_sponge_v2_with_budget(context, clip, budget),
            Self::SpongeV3 => schematic::encode_sponge_v3_with_budget(context, clip, budget),
            Self::Structure => schematic::encode_structure_with_budget(context, clip, budget),
        }
    }
    /// 写入新文件，不修改源结构，不省略不支持的实体、biome 或元数据。
    pub fn write(
        self,
        world: &impl crate::structure_context::CodecContext,
        clip: &BlockClipboard,
        path: &Path,
    ) -> Result<(), WorldError> {
        self.write_with_budget(world, clip, path, EditWorkBudget::default())
    }
    pub fn write_with_budget(
        self,
        world: &impl crate::structure_context::CodecContext,
        clip: &BlockClipboard,
        path: &Path,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        if let Some(source) = &clip.source_file {
            source.verify()?;
        }
        match self {
            Self::Clipboard => clipboard::save_with_budget(world, path, clip, budget),
            Self::Legacy => schematic::export_legacy_with_budget(world, clip, path, budget),
            Self::SpongeV2 => schematic::export_sponge_v2_with_budget(world, clip, path, budget),
            Self::SpongeV3 => schematic::export_sponge_v3_with_budget(world, clip, path, budget),
            Self::Structure => schematic::export_structure_with_budget(world, clip, path, budget),
        }
    }
}
/// 五种格式的新文件导出共用严格创建；编码器不再承担文件替换。
pub(crate) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), WorldError> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let path = std::fs::canonicalize(parent)?.join(
        path.file_name()
            .ok_or_else(|| WorldError::Invalid("结构导出路径没有文件名".into()))?,
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = result {
        if let Err(cleanup) = std::fs::remove_file(&path) {
            log::error!("[structure-export] 清理未完成导出失败：{cleanup}");
        }
        return Err(error.into());
    }
    crate::region_transaction::parent_sync(&path)
}
impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}
