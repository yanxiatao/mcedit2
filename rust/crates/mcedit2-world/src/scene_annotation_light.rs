//! 场景外观附加状态；世界画面统一全亮度，原始光照查询由数据检查入口承担。
use super::*;
use crate::saved_light::Levels;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// 场景显示固定全亮度；Saved 仅表示显式数据检查提供的真实探针。
pub enum Lighting {
    /// 世界、结构和资源的全亮度显示，不请求真实光照。
    #[default]
    Preview,
    /// 与同代场景标记一起完成读取的不可变探针；未知通道仍保留于 Levels。
    Saved {
        probes: Arc<BTreeMap<[i32; 3], Levels>>,
        /// 保存维度定义的实际解释结果；只在需要该语义的探针读取，不伪造天空规则。
        skylight: Result<bool, String>,
    },
}
impl Lighting {
    /// None 表示明确的全亮度显示；显式 Saved 数据的缺失通道仍保持未知。
    pub fn sample(&self, position: [i32; 3]) -> Result<Option<Levels>, WorldError> {
        match self {
            Self::Preview => Ok(None),
            Self::Saved { probes, .. } => probes
                .get(&position)
                .copied()
                .map(Some)
                .ok_or_else(|| invalid(format!("场景光照未捕获所需源格 {position:?}"))),
        }
    }
    pub fn has_skylight(&self) -> Result<Option<bool>, String> {
        match self {
            Self::Preview => Ok(None),
            Self::Saved { skylight, .. } => skylight.clone().map(Some),
        }
    }
}
/// 只采集外观必需的水域和转化状态；显示固定全亮度，不建立光照探针或读取光照邻块。
pub(super) fn attach(
    reader: &mut Reader<'_>,
    annotations: &mut ChunkAnnotations,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), WorldError> {
    super::zombification::attach(reader, annotations, cancelled)?;
    super::water::attach(reader, annotations, cancelled)?;
    annotations.lighting = Lighting::Preview;
    Ok(())
}
