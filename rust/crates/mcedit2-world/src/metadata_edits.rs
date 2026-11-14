//! 世界元数据编辑与已有区块历史的格式约束；普通元数据刷新保留暂存。
use super::*;
fn signature(
    document: &crate::nbt_document::Document,
) -> Result<Vec<Option<crate::nbt_document::Value>>, WorldError> {
    use crate::nbt_document::Value;
    let Value::Compound(data) = document.inspect("/Data")? else {
        return Err(refuse("世界元数据 Data 必须为 Compound"));
    };
    let mut values = ["DataVersion", "version", "Version"]
        .map(|name| {
            data.iter()
                .find(|field| field.name == name)
                .map(|field| field.value.clone())
        })
        .to_vec();
    values.push(
        data.iter()
            .find(|field| field.name == "WorldGenSettings")
            .and_then(|field| match &field.value {
                Value::Compound(fields) => fields
                    .iter()
                    .find(|field| field.name == "dimensions")
                    .map(|field| field.value.clone()),
                _ => Some(field.value.clone()),
            }),
    );
    Ok(values)
}
impl Session {
    /// 已有历史不能跨版本或维度格式转换；普通名称、时间、种子和玩家编辑不受此约束。
    pub(crate) fn verify_metadata_bytes(
        &self,
        _root: &Path,
        path: &Path,
        bytes: &[u8],
    ) -> Result<(), WorldError> {
        let root = &self.world.root;
        if !path.starts_with(root) {
            return Ok(());
        }
        if crate::alpha::is_chunk_path(root, path) {
            return Err(refuse(
                "Alpha 地形文件必须通过区块编辑保存，不能作为独立 NBT 文件覆盖",
            ));
        }
        if !self.has_pending_changes() && self.undo.is_empty() && self.redo.is_empty() {
            return Ok(());
        }
        let primary = root.join("level.dat");
        let source = if primary.is_file() {
            primary
        } else {
            root.join("level.dat_old")
        }
        .canonicalize()?;
        if path != source {
            return Ok(());
        }
        let original = crate::nbt_file::FileDocument::open(root, &source, true)?;
        let (_, decoded) = crate::nbt_envelope::NbtEnvelope::decode(bytes.to_vec())?;
        let next = crate::nbt_document::Document::new(decoded)?;
        if signature(&original.document)? != signature(&next)? {
            return Err(refuse("仍有区块编辑或撤销历史，不能改写世界的数据版本、存储格式或维度定义；请先保存或放弃这些编辑"));
        }
        Ok(())
    }
    /// 刷新已保存的元数据，不清除原来的方块、实体、BE、群系和历史。
    pub fn refresh_metadata(&mut self, world: WorldSummary) -> Result<(), WorldError> {
        if world.root != self.world.root
            || !world
                .dimensions
                .iter()
                .any(|dimension| dimension.name == self.dimension)
        {
            return Err(refuse("刷新元数据属于不同世界或当前维度已移除"));
        }
        let incompatible = world.data_version != self.world.data_version
            || world.storage_format != self.world.storage_format
            || world.version_name != self.world.version_name;
        if incompatible
            && (self.has_pending_changes() || !self.undo.is_empty() || !self.redo.is_empty())
        {
            return Err(refuse("元数据格式已变化，当前编辑历史不能自动转换"));
        }
        let changed = world.world_data.origin != self.world.world_data.origin
            || world.seed != self.world.seed
            || world.level_name != self.world.level_name
            || incompatible;
        self.world = world;
        if changed {
            self.changed_source_chunks
                .extend(self.block_chunk_counts.keys().copied());
            self.note_change();
        }
        Ok(())
    }
}
