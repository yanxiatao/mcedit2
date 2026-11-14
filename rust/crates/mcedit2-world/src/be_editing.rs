//! 选定方块实体内的完整 NBT 编辑证明。
use crate::{
    nbt_document::{patch, Operation},
    sections::{decode_sections, parse_root},
    WorldError,
};

fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    compound_sha256: String,
    position: (i32, i32, i32),
    operation: Operation,
}

/// 两块之间的原始方块实体载荷搬运；变更由混合事务成对验证。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    source_sha256: String,
    target_sha256: [u8; 32],
    source: (i32, i32, i32),
    target: (i32, i32, i32),
    clone: bool,
}

/// 方块实体插入、替换或删除的受限证明；保留本步载荷以顺序重放后续编辑。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Structure {
    position: (i32, i32, i32),
    before_sha: Option<String>,
    after_sha: Option<[u8; 32]>,
    after_payload: Option<Vec<u8>>,
}
impl Structure {
    pub fn position(&self) -> (i32, i32, i32) {
        self.position
    }
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        use sha2::{Digest, Sha256};
        let after_payload = self.after_payload.as_deref();
        let before = crate::sections::inspect_original_block_entity(source, self.position)
            .map_err(|e| invalid(e.to_string()))?;
        if before.as_ref().map(|v| v.original_compound_sha256.as_str())
            != self.before_sha.as_deref()
        {
            return Err(invalid("结构方块实体的原始载荷已变化"));
        }
        if after_payload.map(|v| <[u8; 32]>::from(Sha256::digest(v))) != self.after_sha {
            return Err(invalid("结构方块实体候选摘要不一致"));
        }
        if let Some(payload) = after_payload {
            let mut root = vec![10, 0, 0];
            root.extend_from_slice(payload);
            let tag = parse_root(&root).map_err(|e| invalid(e.to_string()))?;
            for (axis, key) in ["x", "y", "z"].into_iter().enumerate() {
                let expected = [self.position.0, self.position.1, self.position.2][axis];
                if tag.field(key).and_then(|v| v.i32().ok()) != Some(expected) {
                    return Err(invalid("结构方块实体坐标不匹配"));
                }
            }
            let id = tag
                .field("id")
                .ok_or_else(|| invalid("结构方块实体缺少 ID"))?;
            if id.string().map_err(|e| invalid(e.to_string()))?.is_empty() {
                return Err(invalid("结构方块实体 ID 为空"));
            }
        }
        let mut output = source.to_vec();
        let root = parse_root(source).map_err(|e| invalid(e.to_string()))?;
        let modern = root.field("sections").is_some();
        let container = if modern {
            &root
        } else {
            root.field("Level")
                .ok_or_else(|| invalid("区块缺少 Level"))?
        };
        let name = if modern {
            "block_entities"
        } else {
            "TileEntities"
        };
        let list_path = if modern {
            format!("/{name}")
        } else {
            format!("/Level/{name}")
        };
        if let Some(list) = container.field(name) {
            if list.kind != 9 {
                return Err(invalid("方块实体列表类型无效"));
            }
        } else if after_payload.is_some() {
            let parent = if modern { "" } else { "/Level" };
            output = patch(
                &output,
                &Operation::InsertField {
                    path: parent.into(),
                    name: name.into(),
                    value: crate::nbt_document::Value::List(crate::nbt_document::List {
                        element_type: 10,
                        items: Vec::new(),
                    }),
                },
            )?;
        }
        let insert_index = if before.is_some() {
            let path = be_path(&output, self.position)?;
            let index = path
                .rsplit('/')
                .next()
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or_else(|| invalid("方块实体索引无效"))?;
            output = patch(&output, &Operation::Remove { path })?;
            index
        } else {
            let root = parse_root(&output).map_err(|e| invalid(e.to_string()))?;
            let container = if modern {
                &root
            } else {
                root.field("Level")
                    .ok_or_else(|| invalid("区块缺少 Level"))?
            };
            container.field(name).map(|v| v.items.len()).unwrap_or(0)
        };
        if let Some(payload) = after_payload {
            output = crate::nbt_document::insert_raw_compound_list_item(
                &output,
                &list_path,
                insert_index,
                payload,
            )?;
        }
        Ok(output)
    }
}

pub fn structure(
    source: &[u8],
    position: (i32, i32, i32),
    after_payload: Option<&[u8]>,
) -> Result<Structure, WorldError> {
    use sha2::{Digest, Sha256};
    let before = crate::sections::inspect_original_block_entity(source, position)
        .map_err(|e| invalid(e.to_string()))?;
    let proof = Structure {
        position,
        before_sha: before.map(|v| v.original_compound_sha256),
        after_sha: after_payload.map(|v| Sha256::digest(v).into()),
        after_payload: after_payload.map(<[u8]>::to_vec),
    };
    if proof.before_sha.is_none() && proof.after_sha.is_none() {
        return Err(invalid("结构方块实体没有变化"));
    }
    proof.replay(source)?;
    Ok(proof)
}

/// 保留未知字段原字节，仅将方块实体标准坐标变换为目标坐标。
pub fn relocate_compound(payload: &[u8], position: (i32, i32, i32)) -> Result<Vec<u8>, WorldError> {
    let mut bytes = vec![10, 0, 0];
    bytes.extend_from_slice(payload);
    let root = parse_root(&bytes).map_err(|e| invalid(e.to_string()))?;
    let mut operations = Vec::new();
    for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
        let tag = root
            .field(name)
            .ok_or_else(|| invalid("方块实体坐标缺失"))?;
        let current = tag.i32().map_err(|e| invalid(e.to_string()))?;
        let value = [position.0, position.1, position.2][axis];
        if current != value {
            operations.push(Operation::Replace {
                path: format!("/{name}"),
                value: crate::nbt_document::Value::Int(value),
            });
        }
    }
    for op in operations {
        bytes = patch(&bytes, &op)?;
    }
    Ok(bytes[3..].to_vec())
}

impl Transfer {
    pub fn source(&self) -> (i32, i32, i32) {
        self.source
    }
    pub fn target(&self) -> (i32, i32, i32) {
        self.target
    }
    pub fn replay(&self, source: &[u8], target: &[u8]) -> Result<(Vec<u8>, Vec<u8>), WorldError> {
        use sha2::{Digest, Sha256};
        if <[u8; 32]>::from(Sha256::digest(target)) != self.target_sha256 {
            return Err(invalid("方块实体目标已变化"));
        }
        let original = crate::sections::inspect_original_block_entity(source, self.source)
            .map_err(|e| invalid(e.to_string()))?
            .ok_or_else(|| invalid("源方块实体不存在"))?;
        if original.original_compound_sha256 != self.source_sha256 {
            return Err(invalid("源方块实体载荷已变化"));
        }
        if self.source == self.target {
            return Err(invalid("源和目标方块实体位置相同"));
        }
        let source_chunk = (self.source.0.div_euclid(16), self.source.2.div_euclid(16));
        let target_chunk = (self.target.0.div_euclid(16), self.target.2.div_euclid(16));
        if source_chunk == target_chunk && source != target {
            return Err(invalid("同区块的源和目标字节不同"));
        }
        let before = decode_sections(source).map_err(|e| invalid(e.to_string()))?;
        let dest = decode_sections(target).map_err(|e| invalid(e.to_string()))?;
        if before.x.is_some_and(|x| x != source_chunk.0)
            || before.z.is_some_and(|z| z != source_chunk.1)
            || dest.x.is_some_and(|x| x != target_chunk.0)
            || dest.z.is_some_and(|z| z != target_chunk.1)
        {
            return Err(invalid("方块实体区块归属不匹配"));
        }
        if dest.block_entities.contains(&self.target) {
            return Err(invalid("目标已有方块实体"));
        }
        let source_block = before
            .block(
                self.source.0.rem_euclid(16) as u8,
                self.source.1,
                self.source.2.rem_euclid(16) as u8,
            )
            .ok_or_else(|| invalid("源方块不存在"))?;
        let target_block = dest
            .block(
                self.target.0.rem_euclid(16) as u8,
                self.target.1,
                self.target.2.rem_euclid(16) as u8,
            )
            .ok_or_else(|| invalid("目标方块不存在"))?;
        if source_block != target_block {
            return Err(invalid("目标方块与源方块状态不一致，不能假定方块实体兼容"));
        }
        let path = be_path(source, self.source)?;
        let (list, index) = path
            .rsplit_once('/')
            .ok_or_else(|| invalid("方块实体路径无效"))?;
        let index: usize = index.parse().map_err(|_| invalid("方块实体索引无效"))?;
        let mut moved = source.to_vec();
        for (axis, field) in ["x", "y", "z"].into_iter().enumerate() {
            let old = [self.source.0, self.source.1, self.source.2][axis];
            let value = [self.target.0, self.target.1, self.target.2][axis];
            if old != value {
                moved = patch(
                    &moved,
                    &Operation::Replace {
                        path: format!("{path}/{field}"),
                        value: crate::nbt_document::Value::Int(value),
                    },
                )?;
            }
        }
        let payload = crate::nbt_document::raw_compound_list_item(&moved, list, index)?;
        let source_after = if self.clone {
            source.to_vec()
        } else {
            patch(source, &Operation::Remove { path: path.clone() })?
        };
        let target_base = if source_chunk == target_chunk {
            &source_after
        } else {
            target
        };
        let root = parse_root(target_base).map_err(|e| invalid(e.to_string()))?;
        let modern = root.field("sections").is_some();
        let container = if modern {
            &root
        } else {
            root.field("Level")
                .ok_or_else(|| invalid("目标缺少 Level"))?
        };
        let name = if modern {
            "block_entities"
        } else {
            "TileEntities"
        };
        let list_tag = container
            .field(name)
            .ok_or_else(|| invalid("目标缺少方块实体列表"))?;
        if list_tag.kind != 9 {
            return Err(invalid("目标方块实体不是列表"));
        }
        let target_list = if modern {
            format!("/{name}")
        } else {
            format!("/Level/{name}")
        };
        let target_after = crate::nbt_document::insert_raw_compound_list_item(
            target_base,
            &target_list,
            list_tag.items.len(),
            &payload,
        )?;
        if source_chunk == target_chunk {
            Ok((target_after.clone(), target_after))
        } else {
            Ok((source_after, target_after))
        }
    }
}

pub fn transfer(
    source: &[u8],
    target: &[u8],
    from: (i32, i32, i32),
    to: (i32, i32, i32),
    clone: bool,
) -> Result<Transfer, WorldError> {
    use sha2::{Digest, Sha256};
    let original = crate::sections::inspect_original_block_entity(source, from)
        .map_err(|e| invalid(e.to_string()))?
        .ok_or_else(|| invalid("源方块实体不存在"))?;
    let proof = Transfer {
        source_sha256: original.original_compound_sha256,
        target_sha256: Sha256::digest(target).into(),
        source: from,
        target: to,
        clone,
    };
    proof.replay(source, target)?;
    Ok(proof)
}

pub(crate) fn be_path(source: &[u8], position: (i32, i32, i32)) -> Result<String, WorldError> {
    let root = parse_root(source).map_err(|e| invalid(e.to_string()))?;
    let modern = root.field("sections").is_some();
    let container = if modern {
        &root
    } else {
        root.field("Level")
            .ok_or_else(|| invalid("区块缺少 Level"))?
    };
    let name = if modern {
        "block_entities"
    } else {
        "TileEntities"
    };
    let list = container
        .field(name)
        .ok_or_else(|| invalid("区块缺少方块实体列表"))?;
    if list.kind != 9 {
        return Err(invalid("方块实体不是列表"));
    }
    let mut found = None;
    for (index, entity) in list.items.iter().enumerate() {
        if entity.kind != 10 {
            return Err(invalid("方块实体不是 Compound"));
        }
        let coords = ["x", "y", "z"].map(|key| entity.field(key).and_then(|v| v.i32().ok()));
        if coords == [Some(position.0), Some(position.1), Some(position.2)] {
            if found.replace(index).is_some() {
                return Err(invalid("方块实体坐标重复"));
            }
        }
    }
    let index = found.ok_or_else(|| invalid("指定方块实体不存在"))?;
    Ok(if modern {
        format!("/{name}/{index}")
    } else {
        format!("/Level/{name}/{index}")
    })
}

impl Edit {
    pub fn position(&self) -> (i32, i32, i32) {
        self.position
    }
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        let before = decode_sections(source).map_err(|e| invalid(e.to_string()))?;
        if before
            .x
            .is_some_and(|x| x != self.position.0.div_euclid(16))
            || before
                .z
                .is_some_and(|z| z != self.position.2.div_euclid(16))
        {
            return Err(invalid("方块实体不属于当前区块"));
        }
        let prefix = be_path(source, self.position)?;
        let path = match &self.operation {
            Operation::Replace { path, .. }
            | Operation::InsertField { path, .. }
            | Operation::InsertItem { path, .. }
            | Operation::Remove { path }
            | Operation::Rename { path, .. } => path,
        };
        if !path.starts_with(&(prefix.clone() + "/"))
            && !(path == &prefix && matches!(&self.operation, Operation::InsertField { .. }))
        {
            return Err(invalid("NBT 操作超出选中方块实体"));
        }
        let original = crate::sections::inspect_original_block_entity(source, self.position)
            .map_err(|e| invalid(e.to_string()))?
            .ok_or_else(|| invalid("方块实体不存在"))?;
        if original.original_compound_sha256 != self.compound_sha256 {
            return Err(invalid("方块实体编辑源已变化"));
        }
        let output = patch(source, &self.operation)?;
        let after = decode_sections(&output).map_err(|e| invalid(e.to_string()))?;
        if before.block_entities != after.block_entities
            || before.x != after.x
            || before.z != after.z
        {
            return Err(invalid("方块实体编辑改变了坐标或区块归属"));
        }
        let updated = crate::sections::inspect_original_block_entity(&output, self.position)
            .map_err(|e| invalid(e.to_string()))?
            .ok_or_else(|| invalid("编辑删除了方块实体"))?;
        if original.raw_id != updated.raw_id {
            return Err(invalid("方块实体 ID 变化需要方块兼容性验证"));
        }
        Ok(output)
    }
}

pub fn edit(
    source: &[u8],
    position: (i32, i32, i32),
    operation: Operation,
) -> Result<Edit, WorldError> {
    let original = crate::sections::inspect_original_block_entity(source, position)
        .map_err(|e| invalid(e.to_string()))?
        .ok_or_else(|| invalid("方块实体不存在"))?;
    let proof = Edit {
        compound_sha256: original.original_compound_sha256,
        position,
        operation,
    };
    proof.replay(source)?;
    Ok(proof)
}

/// 同一方块实体中的多次文档快照编辑；不重新编码未修改的字段。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditBatch {
    compound_sha256: String,
    position: (i32, i32, i32),
    batches: Vec<Vec<Operation>>,
}
impl EditBatch {
    pub fn position(&self) -> (i32, i32, i32) {
        self.position
    }
    pub fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        let original = crate::sections::inspect_original_block_entity(source, self.position)
            .map_err(|e| invalid(e.to_string()))?
            .ok_or_else(|| invalid("方块实体不存在"))?;
        if original.original_compound_sha256 != self.compound_sha256 {
            return Err(invalid("方块实体批量编辑源已变化"));
        }
        let prefix = be_path(source, self.position)?;
        let output =
            crate::editing::object_editing::replay_batches(source, &prefix, &self.batches, || {
                false
            })?;
        let before = decode_sections(source).map_err(|e| invalid(e.to_string()))?;
        let after = decode_sections(&output).map_err(|e| invalid(e.to_string()))?;
        if before
            .x
            .is_some_and(|x| x != self.position.0.div_euclid(16))
            || before
                .z
                .is_some_and(|z| z != self.position.2.div_euclid(16))
        {
            return Err(invalid("方块实体不属于当前区块"));
        }
        if before.block_entities != after.block_entities
            || before.x != after.x
            || before.z != after.z
        {
            return Err(invalid("方块实体批量编辑改变了坐标或区块归属"));
        }
        let updated = crate::sections::inspect_original_block_entity(&output, self.position)
            .map_err(|e| invalid(e.to_string()))?
            .ok_or_else(|| invalid("编辑删除了方块实体"))?;
        if original.raw_id != updated.raw_id {
            return Err(invalid("方块实体 ID 变化需要方块兼容性验证"));
        }
        Ok(output)
    }
}
pub fn edit_batches(
    source: &[u8],
    position: (i32, i32, i32),
    batches: Vec<Vec<Operation>>,
) -> Result<EditBatch, WorldError> {
    let original = crate::sections::inspect_original_block_entity(source, position)
        .map_err(|e| invalid(e.to_string()))?
        .ok_or_else(|| invalid("方块实体不存在"))?;
    let proof = EditBatch {
        compound_sha256: original.original_compound_sha256,
        position,
        batches,
    };
    proof.replay(source)?;
    Ok(proof)
}
