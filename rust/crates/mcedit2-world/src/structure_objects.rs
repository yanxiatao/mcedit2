//! 独立结构中的对象行与完整根修改；保留文件几何、独立 blockPos 及未知字段。
use super::*;
use crate::editing::{
    script::objects::{self, Row, Source},
    snapshot,
};
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(invalid("结构对象操作已取消"))
    } else {
        Ok(())
    }
}
impl Document {
    pub(crate) fn order_script_objects(
        &mut self,
        tile: bool,
        rows: &[Row],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<Row>, WorldError> {
        self.require_editable()?;
        let mut order = Vec::new();
        for row in rows {
            let actual = self.script_object_row(&row.source, budget, &mut cancelled)?;
            order.push(
                actual
                    .source
                    .path
                    .rsplit('/')
                    .next()
                    .expect("真实对象路径")
                    .parse::<usize>()
                    .map_err(|_| invalid("对象根路径无效"))?,
            );
        }
        let mut next = (*self.current).clone();
        if tile {
            crate::root_order::reorder_slots(&mut next.block_entities, &order)?;
        } else {
            crate::root_order::reorder_slots(&mut next.entities, &order)?;
        }
        stop(&mut cancelled)?;
        self.commit(next, budget, "脚本对象重排")?;
        order.sort_unstable();
        order
            .into_iter()
            .zip(rows)
            .map(|(index, before)| {
                let compound = if tile {
                    &self.current.block_entities[index].compound
                } else {
                    &self.current.entities[index].compound
                };
                if compound != &before.compound {
                    return Err(invalid("结构对象重排回执不同"));
                }
                objects::row(
                    compound,
                    tile,
                    format!(
                        "/{}/{index}",
                        if tile { "block_entities" } else { "Entities" }
                    ),
                    self.current.data_version,
                )
            })
            .collect()
    }
    pub(crate) fn add_script_object(
        &mut self,
        tile: bool,
        nbt: &nbt_document::Value,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Row, WorldError> {
        self.require_editable()?;
        let mut next = (*self.current).clone();
        let mut compound = snapshot::compound(None, nbt)?;
        let object = objects::row(&compound, tile, String::new(), next.data_version)?;
        let local = self
            .script_local_region(
                crate::editing::Bounds {
                    minimum: object.source.position,
                    maximum: object.source.position,
                }
                .into(),
            )?
            .bounds
            .minimum;
        let index;
        if tile {
            let at = (local[1] as usize * usize::from(next.size[2]) + local[2] as usize)
                * usize::from(next.size[0])
                + local[0] as usize;
            crate::be_registry::validate(
                next.data_version,
                &next.blocks[at].name,
                &object.source.id,
            )?;
            index = next
                .block_entities
                .iter()
                .position(|body| body.position == local)
                .unwrap_or(next.block_entities.len());
            let body = crate::clipboard::OpaqueBlockEntity {
                position: local,
                compound,
            };
            if index == next.block_entities.len() {
                next.block_entities.push(body);
            } else {
                let before = nbt_document::Document::new(
                    [
                        &[10, 0, 0][..],
                        next.block_entities[index].compound.as_slice(),
                    ]
                    .concat(),
                )?;
                let after = nbt_document::Document::new(
                    [&[10, 0, 0][..], body.compound.as_slice()].concat(),
                )?;
                if let Some(state) = crate::container_data::coupled_state(
                    &before,
                    &after,
                    &next.blocks[at],
                    crate::inventory::Context {
                        root: self.path.clone(),
                        version: next.data_version,
                    },
                )? {
                    next.blocks[at] = state;
                }
                next.block_entities[index] = body;
            }
        } else {
            let position = snapshot::entity_position(nbt)?;
            let body = crate::clipboard::OpaqueEntity {
                position: std::array::from_fn(|axis| {
                    position[axis] - f64::from(next.source_min[axis])
                }),
                block_position: None,
                compound,
            };
            let (mut compounds, references) = crate::entity_editing::clone_compounds_linked(
                &[body.clone()],
                &next.entities,
                next.data_version,
                [0; 3],
                &crate::entity_editing::DimensionChange::within(next.entity_dimension.as_deref()),
                budget,
                &mut cancelled,
            )?;
            for existing in &mut next.entities {
                if let Some(rewritten) = references.rewrite(
                    &existing.compound,
                    next.data_version,
                    budget,
                    &mut cancelled,
                )? {
                    existing.compound = rewritten;
                }
            }
            compound = compounds.pop().expect("一个新增实体产生一个完整根");
            index = next.entities.len();
            next.entities
                .push(crate::clipboard::OpaqueEntity { compound, ..body });
        }
        if let Some(definitions) = &self.definitions {
            next.bind_paintings(definitions)?;
        }
        crate::entity_editing::validate_document_identities(
            &next.entities,
            budget,
            &mut cancelled,
        )?;
        stop(&mut cancelled)?;
        self.commit(next, budget, "脚本新增对象")?;
        let compound = if tile {
            &self.current.block_entities[index].compound
        } else {
            &self.current.entities[index].compound
        };
        objects::row(
            compound,
            tile,
            format!(
                "/{}/{index}",
                if tile { "block_entities" } else { "Entities" }
            ),
            self.current.data_version,
        )
    }
    pub(crate) fn script_object_rows(
        &self,
        region: SelectionRegion,
        tile: bool,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<Row>, WorldError> {
        self.script_local_region(region)?;
        let mut rows = Vec::new();
        let mut bytes = 0usize;
        let count = if tile {
            self.current.block_entities.len()
        } else {
            self.current.entities.len()
        };
        for index in 0..count {
            stop(&mut cancelled)?;
            let compound = if tile {
                &self.current.block_entities[index].compound
            } else {
                &self.current.entities[index].compound
            };
            let row = objects::row(
                compound,
                tile,
                format!(
                    "/{}/{index}",
                    if tile { "block_entities" } else { "Entities" }
                ),
                self.current.data_version,
            )?;
            let selected = if tile {
                region.contains(row.source.position.into())
            } else {
                region.contains_point(snapshot::entity_position(&row.nbt)?)
            };
            if !selected {
                continue;
            }
            bytes = bytes
                .checked_add(row.bytes)
                .ok_or_else(|| invalid("结构对象查询工作集溢出"))?;
            budget.check(bytes.saturating_mul(16))?;
            rows.push(row);
        }
        Ok(rows)
    }
    pub(crate) fn script_object_row(
        &self,
        source: &Source,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Row, WorldError> {
        self.script_object_row_optional(source, budget, &mut cancelled)?
            .ok_or_else(|| invalid("结构对象已移除或实际身份已改变"))
    }
    pub(crate) fn script_object_row_optional(
        &self,
        source: &Source,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Row>, WorldError> {
        let mut rows = self
            .script_object_rows(
                self.script_bounds()?.into(),
                source.tile,
                budget,
                &mut cancelled,
            )?
            .into_iter()
            .filter(|row| {
                if source.tile {
                    source.position == row.source.position && source.id == row.source.id
                } else if let Some(uuid) = source.uuid {
                    row.source.uuid == Some(uuid)
                } else {
                    source.path == row.source.path && source.sha256 == row.source.sha256
                }
            });
        let row = rows.next();
        if rows.next().is_some() {
            return Err(invalid("结构对象身份重复，须解决来源冲突"));
        }
        Ok(row)
    }
    pub(crate) fn change_script_object(
        &mut self,
        before: &Row,
        after: Option<&nbt_document::Value>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Row>, WorldError> {
        self.require_editable()?;
        let current = self.script_object_row(&before.source, budget, &mut cancelled)?;
        if current.nbt != before.nbt {
            return Err(invalid("结构对象内容已改变，不能覆盖"));
        }
        let index: usize = current
            .source
            .path
            .rsplit('/')
            .next()
            .expect("真实根路径")
            .parse()
            .map_err(|_| invalid("结构对象根路径无效"))?;
        let mut next = (*self.current).clone();
        if before.source.tile {
            next.block_entities.remove(index);
            if let Some(after) = after {
                let position = snapshot::be_position(after)?;
                let local = self
                    .script_local_region(
                        crate::editing::Bounds {
                            minimum: position,
                            maximum: position,
                        }
                        .into(),
                    )?
                    .bounds
                    .minimum;
                if next
                    .block_entities
                    .iter()
                    .any(|body| body.position == local)
                {
                    return Err(invalid("结构方块实体目标已被占用"));
                }
                let compound = snapshot::compound(Some(&current.compound), after)?;
                let raw = [&[10, 0, 0][..], compound.as_slice()].concat();
                let root = parse_root(&raw).map_err(invalid)?;
                let id = root
                    .require("id", 8)
                    .and_then(Tag::string)
                    .map_err(invalid)?;
                let at = (local[1] as usize * usize::from(next.size[2]) + local[2] as usize)
                    * usize::from(next.size[0])
                    + local[0] as usize;
                crate::be_registry::validate(next.data_version, &next.blocks[at].name, &id)?;
                if position == before.source.position {
                    let before_doc = nbt_document::Document::new(
                        [&[10, 0, 0][..], current.compound.as_slice()].concat(),
                    )?;
                    let after_doc = nbt_document::Document::new(raw.clone())?;
                    let context = crate::inventory::Context {
                        root: self.path.clone(),
                        version: next.data_version,
                    };
                    if let Some(state) = crate::container_data::coupled_state(
                        &before_doc,
                        &after_doc,
                        &next.blocks[at],
                        context,
                    )? {
                        next.blocks[at] = state;
                    }
                }
                next.block_entities.insert(
                    index,
                    crate::clipboard::OpaqueBlockEntity {
                        position: local,
                        compound,
                    },
                );
            }
        } else {
            let previous = next.entities.remove(index);
            if let Some(after) = after {
                let compound = snapshot::compound(Some(&current.compound), after)?;
                let object = objects::row(
                    &compound,
                    false,
                    current.source.path.clone(),
                    next.data_version,
                )?;
                let absolute = snapshot::entity_position(after)?;
                let cell = object.source.position;
                self.script_local_region(
                    crate::editing::Bounds {
                        minimum: cell,
                        maximum: cell,
                    }
                    .into(),
                )?;
                next.entities.insert(
                    index,
                    crate::clipboard::OpaqueEntity {
                        position: std::array::from_fn(|axis| {
                            absolute[axis] - f64::from(next.source_min[axis])
                        }),
                        block_position: previous.block_position,
                        compound,
                    },
                );
            }
            crate::entity_editing::validate_document_identities(
                &next.entities,
                budget,
                &mut cancelled,
            )?;
        }
        stop(&mut cancelled)?;
        if let Some(definitions) = &self.definitions {
            next.bind_paintings(definitions)?;
        }
        self.commit(next, budget, "脚本对象")?;
        after
            .map(|_| {
                let compound = if before.source.tile {
                    &self.current.block_entities[index].compound
                } else {
                    &self.current.entities[index].compound
                };
                objects::row(
                    compound,
                    before.source.tile,
                    current.source.path,
                    self.current.data_version,
                )
            })
            .transpose()
    }
}
