//! 光标位置的真实方块与存储群系；一次读取一列，不生成缺失数据。
use crate::{
    block_lookup::ReadOnlyRegionCache,
    editing::Session,
    sections::{Chunk, State},
    WorldError,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Cell {
    MissingChunk,
    MissingSection,
    Present(State),
}
#[derive(Clone, Debug)]
pub enum Biome {
    Named(String),
    Numeric(i32),
    NumericUnavailable,
    NotStored,
}
#[derive(Clone, Debug)]
pub struct Sample {
    pub block: Cell,
    pub biome: Biome,
}

/// 一个不可变会话代次的一列；由调用方在离开该列或来源变化时释放。
#[derive(Debug)]
pub struct Column {
    coordinate: (i32, i32),
    revision: Arc<()>,
    blocks: Option<Arc<Chunk>>,
    biomes: Option<Arc<Chunk>>,
}
impl Column {
    pub fn coordinate(&self) -> (i32, i32) {
        self.coordinate
    }
    pub fn read(session: &Session, coordinate: (i32, i32)) -> Result<Self, WorldError> {
        let (cx, cz) = coordinate;
        let mut reader = ReadOnlyRegionCache::new(session.original_world(), session.dimension())?;
        let (blocks, _) = session.visible_chunk_base_shared_cached(&mut reader, cx, cz)?;
        let biomes = session
            .staged_biome_chunk_cached(&mut reader, cx, cz)?
            .map(crate::sections::decode_sections)
            .transpose()
            .map_err(|error| WorldError::Invalid(format!("光标群系列无效：{error}")))?
            .map(Arc::new)
            .or_else(|| blocks.clone());
        Ok(Self {
            coordinate,
            revision: session.revision(),
            blocks,
            biomes,
        })
    }
    /// 返回存档的原始群系单元；颜色用的模糊采样不替代状态栏数据。
    pub fn sample(
        &self,
        session: &Session,
        position: (i32, i32, i32),
    ) -> Result<Sample, WorldError> {
        if !Arc::ptr_eq(&self.revision, &session.revision())
            || self.coordinate != (position.0.div_euclid(16), position.2.div_euclid(16))
        {
            return Err(WorldError::Invalid("光标列不属于当前会话代次或位置".into()));
        }
        let (x, y, z) = (
            position.0.rem_euclid(16) as u8,
            position.1,
            position.2.rem_euclid(16) as u8,
        );
        let block = match &self.blocks {
            None => Cell::MissingChunk,
            Some(chunk) => match chunk.block(x, y, z) {
                Some(state) => Cell::Present(session.effective_state(position, state)),
                None => Cell::MissingSection,
            },
        };
        let biome = match &self.biomes {
            Some(chunk) => match (chunk.biome(x, y, z), chunk.numeric_biomes.as_ref()) {
                (Some(name), None) => Biome::Named(name.to_owned()),
                (None, Some(data)) => match data.sample(x, y, z) {
                    Some(id) => Biome::Numeric(id),
                    None => Biome::NumericUnavailable,
                },
                (None, None) => Biome::NotStored,
                (Some(_), Some(_)) => {
                    return Err(WorldError::Invalid("光标位置同时声明命名和数字群系".into()))
                }
            },
            None => Biome::NotStored,
        };
        Ok(Sample { block, biome })
    }
}
