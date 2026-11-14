//! 结构编解码所需的真实来源信息；独立文档不需要地形目录或伪造世界摘要。
use crate::{WorldError, WorldSummary};
use std::path::{Path, PathBuf};

/// 世界导入与独立结构文件共用编解码器，目标世界规则仅在真正存在目标时执行。
pub trait CodecContext {
    /// 原始世界根目录或独立文件的规范路径，用于记录实际来源身份。
    fn identity(&self) -> &Path;
    /// 保存或用户明确选择的源版本，不按相近版本补齐。
    fn data_version(&self) -> Option<i32>;
    /// 来源确实记录的发行版名称。
    fn release(&self) -> Option<&str>;
    /// 真正的导入目标；独立结构编辑没有目标世界。
    fn target_world(&self) -> Option<&WorldSummary>;
    /// 普通世界导出不得落入受管理的存档；独立文件沿文件保存职责处理路径。
    fn exports_into_world(&self, parent: &Path) -> Result<bool, WorldError> {
        match self.target_world() {
            Some(world) => {
                Ok(std::fs::canonicalize(parent)?.starts_with(std::fs::canonicalize(&world.root)?))
            }
            None => Ok(false),
        }
    }
}
impl CodecContext for WorldSummary {
    fn identity(&self) -> &Path {
        &self.root
    }
    fn data_version(&self) -> Option<i32> {
        self.data_version
    }
    fn release(&self) -> Option<&str> {
        self.version_name.as_deref()
    }
    fn target_world(&self) -> Option<&WorldSummary> {
        Some(self)
    }
}

/// 独立文档只携带真实文件路径及版本；不提供地形、维度注册表或目标世界。
#[derive(Debug, Clone)]
pub struct DocumentContext {
    source: PathBuf,
    version: Option<i32>,
    release: Option<String>,
}
impl DocumentContext {
    pub fn new(
        source: &Path,
        version: Option<i32>,
        release: Option<String>,
    ) -> Result<Self, WorldError> {
        Ok(Self {
            source: std::fs::canonicalize(source)?,
            version,
            release,
        })
    }
}
impl CodecContext for DocumentContext {
    fn identity(&self) -> &Path {
        &self.source
    }
    fn data_version(&self) -> Option<i32> {
        self.version
    }
    fn release(&self) -> Option<&str> {
        self.release.as_deref()
    }
    fn target_world(&self) -> Option<&WorldSummary> {
        None
    }
}
