//! 槽位库存的版本化物品编辑；使用原文档世代及完整 Compound，不重建未知载荷。
use crate::{
    nbt_document::{Document, Field, List, Operation, Value},
    WorldError,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
fn error(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Player,
    Ender,
    Chest,
    Dispenser,
    Hopper,
    Furnace,
    Brewing,
    Bookshelf,
    Crafter,
    Campfire,
    Jukebox,
    Lectern,
    Pot,
    Brushable,
    EntityItem,
    EntityArmor,
    EntityHands,
    EntityBody,
    EntityAnimalArmor,
    EntityDecor,
    EntitySaddle,
}
impl Layout {
    /// 原版 PlayerPanel 和三个 GenericContainerEditor 的布局；空格不代表已有物品。
    pub fn rows(self) -> Vec<Vec<i16>> {
        match self {
            Self::Player => {
                let mut rows = vec![vec![100, 101, 102, 103, -106]];
                rows.extend((1..4).map(|row| (row * 9..row * 9 + 9).collect()));
                rows.push((0..9).collect());
                rows
            }
            Self::Ender | Self::Chest => {
                (0..3).map(|row| (row * 9..row * 9 + 9).collect()).collect()
            }
            Self::Dispenser => (0..3).map(|row| (row * 3..row * 3 + 3).collect()).collect(),
            Self::Hopper => vec![(0..5).collect()],
            Self::Furnace => vec![vec![0], vec![1], vec![2]],
            Self::Brewing => vec![vec![4, 3], vec![0, 1, 2]],
            Self::Bookshelf => vec![vec![0, 1, 2], vec![3, 4, 5]],
            Self::Crafter => (0..3).map(|row| (row * 3..row * 3 + 3).collect()).collect(),
            Self::Campfire => vec![vec![0, 1], vec![2, 3]],
            Self::EntityArmor => vec![vec![3, 2, 1, 0]],
            Self::EntityHands => vec![vec![0, 1]],
            Self::Jukebox
            | Self::Lectern
            | Self::Pot
            | Self::Brushable
            | Self::EntityItem
            | Self::EntityBody
            | Self::EntityAnimalArmor
            | Self::EntityDecor
            | Self::EntitySaddle => {
                vec![vec![0]]
            }
        }
    }
    pub fn contains(self, slot: i16) -> bool {
        match self {
            Self::Player => (0..36).contains(&slot) || (100..104).contains(&slot) || slot == -106,
            Self::Chest | Self::Ender => (0..27).contains(&slot),
            Self::Dispenser => (0..9).contains(&slot),
            Self::Hopper | Self::Brewing => (0..5).contains(&slot),
            Self::Furnace => (0..3).contains(&slot),
            Self::Bookshelf => (0..6).contains(&slot),
            Self::Crafter => (0..9).contains(&slot),
            Self::Campfire => (0..4).contains(&slot),
            Self::EntityArmor => (0..4).contains(&slot),
            Self::EntityHands => (0..2).contains(&slot),
            Self::Jukebox
            | Self::Lectern
            | Self::Pot
            | Self::Brushable
            | Self::EntityItem
            | Self::EntityBody
            | Self::EntityAnimalArmor
            | Self::EntityDecor
            | Self::EntitySaddle => slot == 0,
        }
    }
    pub fn single(self) -> bool {
        matches!(
            self,
            Self::Jukebox
                | Self::Lectern
                | Self::Pot
                | Self::Brushable
                | Self::EntityItem
                | Self::EntityBody
                | Self::EntityAnimalArmor
                | Self::EntityDecor
                | Self::EntitySaddle
        )
    }
    /// 生物装备按列表下标确定位置，空槽写空 Compound，不能删除并使后续装备移位。
    pub fn fixed_len(self) -> Option<usize> {
        match self {
            Self::EntityArmor => Some(4),
            Self::EntityHands => Some(2),
            _ => None,
        }
    }
    pub fn equipment(self) -> bool {
        matches!(
            self,
            Self::EntityArmor
                | Self::EntityHands
                | Self::EntityBody
                | Self::EntityAnimalArmor
                | Self::EntityDecor
                | Self::EntitySaddle
        )
    }
    pub fn field(self) -> &'static str {
        match self {
            Self::EntityItem => "Item",
            Self::EntityArmor => "ArmorItems",
            Self::EntityHands => "HandItems",
            Self::EntityBody => "body_armor_item",
            Self::EntityAnimalArmor => "ArmorItem",
            Self::EntityDecor => "DecorItem",
            Self::EntitySaddle => "SaddleItem",
            Self::Jukebox => "RecordItem",
            Self::Lectern => "Book",
            Self::Pot | Self::Brushable => "item",
            _ => "Items",
        }
    }
    pub fn loot(self) -> bool {
        matches!(
            self,
            Self::Chest | Self::Dispenser | Self::Hopper | Self::Pot | Self::Brushable
        )
    }
    /// 以原版槽位间距为单位保留输入、燃料、输出和药瓶的实际排列。
    pub fn positions(self) -> Vec<(i16, u16, u16)> {
        match self {
            Self::Furnace => vec![(0, 0, 0), (1, 0, 36), (2, 60, 18)],
            Self::Brewing => vec![(4, 0, 0), (3, 62, 0), (0, 39, 34), (1, 62, 41), (2, 85, 34)],
            _ => self
                .rows()
                .into_iter()
                .enumerate()
                .flat_map(|(y, row)| {
                    row.into_iter()
                        .enumerate()
                        .map(move |(x, slot)| (slot, x as u16 * 18, y as u16 * 18))
                })
                .collect(),
        }
    }
    /// 掉落物和展示框拥有一个完整 Item Compound，不能当成带 Slot 的容器列表。
    pub fn entity(id: &str) -> Option<Self> {
        (crate::entity_catalog::is_dropped(id) || crate::hanging::is_frame(id))
            .then_some(Self::EntityItem)
    }
    pub fn container(id: &str) -> Option<Self> {
        match id {
            "Chest"
            | "minecraft:chest"
            | "minecraft:trapped_chest"
            | "minecraft:barrel"
            | "minecraft:shulker_box" => Some(Self::Chest),
            "Trap" | "Dropper" | "minecraft:dispenser" | "minecraft:dropper" => {
                Some(Self::Dispenser)
            }
            "Hopper" | "minecraft:hopper" => Some(Self::Hopper),
            "Furnace" | "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker" => {
                Some(Self::Furnace)
            }
            "Cauldron" | "minecraft:brewing_stand" => Some(Self::Brewing),
            "minecraft:chiseled_bookshelf" => Some(Self::Bookshelf),
            "minecraft:crafter" => Some(Self::Crafter),
            "minecraft:campfire" => Some(Self::Campfire),
            "RecordPlayer" | "minecraft:jukebox" => Some(Self::Jukebox),
            "minecraft:lectern" => Some(Self::Lectern),
            "minecraft:decorated_pot" => Some(Self::Pot),
            "minecraft:brushable_block" => Some(Self::Brushable),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub root: PathBuf,
    pub version: Option<i32>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub path: String,
    pub layout: Layout,
    pub context: Context,
}
#[derive(Debug, Clone)]
pub struct Stack {
    pub value: Value,
    pub context: Context,
}
#[derive(Debug, Clone)]
pub struct Entry {
    pub index: usize,
    pub id: String,
    pub count: i32,
    pub damage: Option<i16>,
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub spec: Spec,
    revision: Arc<()>,
    present: bool,
    pub loot_table: Option<String>,
    pub disabled: std::collections::BTreeSet<i16>,
    loot_seed: bool,
    disabled_present: bool,
    pub items: BTreeMap<i16, Entry>,
    list: List,
}
fn fields(value: &Value) -> Result<&[Field], WorldError> {
    match value {
        Value::Compound(fields) => Ok(fields),
        _ => Err(error("物品必须为 Compound")),
    }
}
fn field<'a>(fields: &'a [Field], name: &str) -> Result<Option<&'a Value>, WorldError> {
    let mut found = fields.iter().filter(|field| field.name == name);
    let value = found.next().map(|field| &field.value);
    if found.next().is_some() {
        return Err(error(format!("物品字段 {name} 重复")));
    }
    Ok(value)
}
fn validate_version(value: &Value, version: Option<i32>) -> Result<(), WorldError> {
    let data = fields(value)?;
    if matches!(version, Some(1343 | 3465 | 3955)) {
        let Some(Value::String(id)) = field(data, "id")? else {
            return Err(error("该数据版本的物品 id 必须为 String"));
        };
        validate_id(id)?;
        let valid = if version == Some(3955) {
            matches!(
                (field(data, "Count")?, field(data, "count")?),
                (None, Some(Value::Int(_)))
            )
        } else {
            matches!(
                (field(data, "Count")?, field(data, "count")?),
                (Some(Value::Byte(_)), None)
            )
        };
        if !valid {
            return Err(error("物品数量字段与实际数据版本不符"));
        }
    }
    Ok(())
}
fn set(fields: &mut Vec<Field>, name: &str, value: Value) {
    if let Some(field) = fields.iter_mut().find(|field| field.name == name) {
        field.value = value;
    } else {
        fields.push(Field {
            name: name.into(),
            value,
        });
    }
}
fn pointer(path: &str) -> Result<(&str, String), WorldError> {
    let names = crate::nbt_document::parse_path(path)?;
    let name = names
        .last()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| error("库存必须位于非空 Compound 字段"))?
        .clone();
    let (parent, _) = path
        .rsplit_once('/')
        .ok_or_else(|| error("库存路径缺少字段"))?;
    Ok((parent, name))
}
fn decode(value: &Value, index: usize, layout: Layout) -> Result<(i16, Entry), WorldError> {
    let data = fields(value)?;
    let slot = if layout.single() {
        0
    } else if layout.fixed_len().is_some() {
        i16::try_from(index).map_err(|_| error("装备列表下标超出槽位范围"))?
    } else {
        match field(data, "Slot")? {
            Some(Value::Byte(slot)) => i16::from(*slot),
            _ => return Err(error("物品 Slot 必须为 Byte")),
        }
    };
    if !layout.contains(slot) {
        return Err(error(format!("槽 {slot} 不属于当前库存布局")));
    }
    let id = match field(data, "id")? {
        Some(Value::String(id)) => id.clone(),
        Some(Value::Short(id)) => id.to_string(),
        _ => return Err(error("物品 id 必须为 String 或旧版 Short")),
    };
    let count = match (field(data, "Count")?, field(data, "count")?) {
        (Some(Value::Byte(n)), None) => i32::from(*n),
        (None, Some(Value::Int(n))) => *n,
        _ => return Err(error("物品必须有唯一的 Count Byte 或 count Int")),
    };
    let damage = match field(data, "Damage")? {
        None => None,
        Some(Value::Short(value)) => Some(*value),
        _ => return Err(error("旧版 Damage 必须为 Short")),
    };
    Ok((
        slot,
        Entry {
            index,
            id,
            count,
            damage,
        },
    ))
}
/// 读取完整版本化物品；空 Compound、air 与非正数量是实际空槽，不合成替代物品。
/// 装备解析和库存编辑共用此入口，附加组件及未知字段仍由调用方保留原载荷。
pub fn read_stack(value: &Value, version: Option<i32>) -> Result<Option<Entry>, WorldError> {
    if fields(value)?.is_empty() {
        return Ok(None);
    }
    validate_version(value, version)?;
    let (_, entry) = decode(value, 0, Layout::EntityItem)?;
    Ok((entry.id != "minecraft:air" && entry.count > 0).then_some(entry))
}
impl Snapshot {
    /// 只读借用真实物品载荷，供图标读取组件和旧版标签，不复制整个库存。
    pub fn item_value(&self, slot: i16) -> Option<&Value> {
        self.items
            .get(&slot)
            .map(|entry| &self.list.items[entry.index])
    }

    pub fn capture(doc: &Document, spec: Spec) -> Result<Self, WorldError> {
        if spec.layout == Layout::EntityBody && spec.context.version != Some(3955) {
            return Err(error("body_armor_item 尚未绑定当前数据版本"));
        }
        if matches!(spec.layout, Layout::EntityAnimalArmor | Layout::EntityDecor)
            && !matches!(spec.context.version, Some(1343 | 3465))
        {
            return Err(error("动物旧装备字段与实际版本不同"));
        }
        let (parent, name) = pointer(&spec.path)?;
        let value = doc.inspect_field(parent, &name)?;
        let present = value.is_some();
        let list = match value {
            None => List {
                element_type: 10,
                items: Vec::new(),
            },
            Some(value @ Value::Compound(_)) if spec.layout.single() => {
                let empty = read_stack(&value, spec.context.version)?.is_none();
                List {
                    element_type: 10,
                    items: if empty { Vec::new() } else { vec![value] },
                }
            }
            Some(Value::List(list))
                if !spec.layout.single()
                    && (list.element_type == 10
                        || (list.element_type == 0 && list.items.is_empty())) =>
            {
                list
            }
            _ => return Err(error("库存必须为 Compound 列表")),
        };
        if spec
            .layout
            .fixed_len()
            .is_some_and(|length| list.items.len() > length)
        {
            return Err(error("装备列表超出实际槽位数量"));
        }
        let container = spec.layout.loot();
        let loot_table = if container {
            match doc.inspect_field(parent, "LootTable")? {
                None => None,
                Some(Value::String(id)) => Some(id),
                _ => return Err(error("容器 LootTable 必须为 String")),
            }
        } else {
            None
        };
        let loot_seed = if loot_table.is_some() {
            match doc.inspect_field(parent, "LootTableSeed")? {
                None => false,
                Some(Value::Long(_)) => true,
                _ => return Err(error("容器 LootTableSeed 必须为 Long")),
            }
        } else {
            false
        };
        let (inventory_parent, _) = pointer(&spec.path)?;
        let disabled_present = spec.layout == Layout::Crafter
            && doc
                .inspect_field(inventory_parent, "disabled_slots")?
                .is_some();
        let disabled = if spec.layout == Layout::Crafter {
            let (parent, _) = pointer(&spec.path)?;
            match doc.inspect_field(parent, "disabled_slots")? {
                None => std::collections::BTreeSet::new(),
                Some(Value::IntArray(values)) => values
                    .into_iter()
                    .map(|n| {
                        if (0..9).contains(&n) {
                            Ok(n as i16)
                        } else {
                            Err(error("合成器禁用槽位越界"))
                        }
                    })
                    .collect::<Result<_, _>>()?,
                _ => return Err(error("合成器 disabled_slots 必须为 IntArray")),
            }
        } else {
            std::collections::BTreeSet::new()
        };
        let mut items = BTreeMap::new();
        for (index, value) in list.items.iter().enumerate() {
            if spec.layout.fixed_len().is_some()
                && read_stack(value, spec.context.version)?.is_none()
            {
                continue;
            }
            validate_version(value, spec.context.version)?;
            let (slot, entry) = decode(value, index, spec.layout)?;
            if items.insert(slot, entry).is_some() {
                return Err(error(format!("库存槽 {slot} 重复")));
            }
        }
        Ok(Self {
            revision: doc.revision(),
            spec,
            present,
            loot_table,
            loot_seed,
            disabled,
            disabled_present,
            items,
            list,
        })
    }
    pub fn current(&self, doc: &Document) -> bool {
        Arc::ptr_eq(&self.revision, &doc.revision())
    }
    pub fn verify(&self, doc: &Document) -> Result<(), WorldError> {
        if !self.current(doc) {
            return Err(error("库存文档或内容已改变，请重新选择槽位"));
        }
        Ok(())
    }
    pub fn item_path(&self, slot: i16) -> Result<String, WorldError> {
        let entry = self.items.get(&slot).ok_or_else(|| error("所选槽为空"))?;
        Ok(if self.spec.layout.single() {
            self.spec.path.clone()
        } else {
            format!("{}/{}", self.spec.path, entry.index)
        })
    }
    pub fn copy(&self, slot: i16) -> Result<Stack, WorldError> {
        Ok(Stack {
            value: self.list.items[self
                .items
                .get(&slot)
                .ok_or_else(|| error("所选槽为空"))?
                .index]
                .clone(),
            context: self.spec.context.clone(),
        })
    }
    fn editable(&self) -> Result<(), WorldError> {
        if self.loot_table.is_some() {
            return Err(error("容器仍使用战利品表，请先明确改为手动物品"));
        }
        Ok(())
    }
    pub fn make_manual(&self) -> Result<Vec<Operation>, WorldError> {
        if self.loot_table.is_none() {
            return Err(error("容器没有战利品表"));
        }
        let (parent, _) = pointer(&self.spec.path)?;
        let mut operations = vec![Operation::Remove {
            path: format!("{parent}/LootTable"),
        }];
        if self.loot_seed {
            operations.push(Operation::Remove {
                path: format!("{parent}/LootTableSeed"),
            });
        }
        Ok(operations)
    }
    fn operation(&self, mut list: List) -> Result<Vec<Operation>, WorldError> {
        if let Some(length) = self.spec.layout.fixed_len() {
            if list.items.len() > length {
                return Err(error("装备列表超出实际槽位数量"));
            }
            list.items
                .resize_with(length, || Value::Compound(Vec::new()));
        }
        if !list.items.is_empty() {
            list.element_type = 10;
        }
        if self.spec.layout.single() {
            if let Some(value) = list.items.into_iter().next() {
                return self.place(0, value);
            }
            if self.spec.layout == Layout::Brushable && self.spec.context.version == Some(3465) {
                return Ok(vec![Operation::Replace {
                    path: self.spec.path.clone(),
                    value: Value::Compound(vec![
                        Field {
                            name: "id".into(),
                            value: Value::String("minecraft:air".into()),
                        },
                        Field {
                            name: "Count".into(),
                            value: Value::Byte(0),
                        },
                    ]),
                }]);
            }
            return Ok(if self.present {
                vec![Operation::Remove {
                    path: self.spec.path.clone(),
                }]
            } else {
                Vec::new()
            });
        }
        let value = Value::List(list);
        Ok(vec![if self.present {
            Operation::Replace {
                path: self.spec.path.clone(),
                value,
            }
        } else {
            let (parent, name) = pointer(&self.spec.path)?;
            Operation::InsertField {
                path: parent.into(),
                name,
                value,
            }
        }])
    }
    fn place(&self, slot: i16, mut value: Value) -> Result<Vec<Operation>, WorldError> {
        self.editable()?;
        if !self.spec.layout.contains(slot) {
            return Err(error("目标槽不属于当前库存布局"));
        }
        let Value::Compound(data) = &mut value else {
            return Err(error("物品必须为 Compound"));
        };
        if self.disabled.contains(&slot) {
            return Err(error("目标合成器槽位已禁用"));
        }
        if self.spec.layout.single() || self.spec.layout.fixed_len().is_some() {
            data.retain(|f| f.name != "Slot");
        } else {
            set(
                data,
                "Slot",
                Value::Byte(i8::try_from(slot).map_err(|_| error("槽位超出 Byte 范围"))?),
            );
        }
        decode(&value, 0, self.spec.layout)?;
        validate_version(&value, self.spec.context.version)?;
        if self.item_value(slot) == Some(&value) {
            return Ok(Vec::new());
        }
        if let Some(length) = self.spec.layout.fixed_len() {
            let mut list = self.list.clone();
            list.items
                .resize_with(length, || Value::Compound(Vec::new()));
            list.items[slot as usize] = value;
            return self.operation(list);
        }
        if self.spec.layout.single() {
            let (parent, name) = pointer(&self.spec.path)?;
            return Ok(vec![if self.present {
                Operation::Replace {
                    path: self.spec.path.clone(),
                    value,
                }
            } else {
                Operation::InsertField {
                    path: parent.into(),
                    name,
                    value,
                }
            }]);
        }
        if let Some(entry) = self.items.get(&slot) {
            return Ok(vec![Operation::Replace {
                path: format!("{}/{}", self.spec.path, entry.index),
                value,
            }]);
        }
        if self.present {
            return Ok(vec![Operation::InsertItem {
                path: self.spec.path.clone(),
                index: self.list.items.len(),
                value,
            }]);
        }
        self.operation(List {
            element_type: 10,
            items: vec![value],
        })
    }
    /// ID / 数量 / 旧版耐久一次应用；其余 tag、components 和未知字段原样保留。
    pub fn edit(
        &self,
        slot: i16,
        id: &str,
        count: i32,
        damage: Option<i16>,
    ) -> Result<Vec<Operation>, WorldError> {
        self.editable()?;
        let value = if let Some(entry) = self.items.get(&slot) {
            self.list.items[entry.index].clone()
        } else {
            new_stack(id.trim(), count, self.spec.context.version)?
        };
        let value = edit_fields(value, self.spec.context.version, id, count, damage)?;
        if let Some(entry) = self.items.get(&slot) {
            let before = fields(&self.list.items[entry.index])?;
            let after = fields(&value)?;
            let mut ops = Vec::new();
            for name in ["id", "Count", "count", "Damage"] {
                if let Some(value) = field(after, name)? {
                    if field(before, name)? != Some(value) {
                        ops.push(if field(before, name)?.is_some() {
                            Operation::Replace {
                                path: format!("{}/{name}", self.item_path(slot)?),
                                value: value.clone(),
                            }
                        } else {
                            Operation::InsertField {
                                path: self.item_path(slot)?,
                                name: name.into(),
                                value: value.clone(),
                            }
                        });
                    }
                }
            }
            return Ok(ops);
        }
        self.place(slot, value)
    }
    pub fn remove(&self, slot: i16) -> Result<Vec<Operation>, WorldError> {
        self.editable()?;
        if self.spec.layout.fixed_len().is_some() {
            let entry = self.items.get(&slot).ok_or_else(|| error("所选槽为空"))?;
            let mut list = self.list.clone();
            list.items[entry.index] = Value::Compound(Vec::new());
            return self.operation(list);
        }
        if self.spec.layout.single() {
            self.item_path(slot)?;
            return self.operation(List {
                element_type: 10,
                items: Vec::new(),
            });
        }
        Ok(vec![Operation::Remove {
            path: self.item_path(slot)?,
        }])
    }
    pub fn clear(&self) -> Result<Vec<Operation>, WorldError> {
        self.editable()?;
        if self.items.is_empty() {
            return Err(error("库存已经为空"));
        }
        self.operation(List {
            element_type: self.list.element_type,
            items: Vec::new(),
        })
    }
    pub fn swap(&self, from: i16, to: i16) -> Result<Vec<Operation>, WorldError> {
        self.editable()?;
        if from == to {
            return Err(error("源槽和目标槽相同"));
        }
        if !self.spec.layout.contains(to) {
            return Err(error("目标槽不属于当前库存布局"));
        }
        if self.disabled.contains(&to) {
            return Err(error("目标合成器槽位已禁用"));
        }
        let source = self.items.get(&from).ok_or_else(|| error("源槽为空"))?;
        if let Some(length) = self.spec.layout.fixed_len() {
            let mut list = self.list.clone();
            list.items
                .resize_with(length, || Value::Compound(Vec::new()));
            list.items.swap(source.index, to as usize);
            return self.operation(list);
        }
        let mut ops = Vec::new();
        for (entry, slot) in [(Some(source), to), (self.items.get(&to), from)] {
            if let Some(entry) = entry {
                ops.push(Operation::Replace {
                    path: format!("{}/{}/Slot", self.spec.path, entry.index),
                    value: Value::Byte(
                        i8::try_from(slot).map_err(|_| error("槽位超出 Byte 范围"))?,
                    ),
                });
            }
        }
        Ok(ops)
    }
    pub fn set_disabled(&self, slot: i16, disabled: bool) -> Result<Vec<Operation>, WorldError> {
        if self.spec.layout != Layout::Crafter || !self.spec.layout.contains(slot) {
            return Err(error("这不是合成器有效槽位"));
        }
        if self.disabled.contains(&slot) == disabled {
            return Ok(Vec::new());
        }
        if disabled && self.items.contains_key(&slot) {
            return Err(error("合成器只能禁用空槽"));
        }
        let (parent, _) = pointer(&self.spec.path)?;
        let mut values = self.disabled.clone();
        if disabled {
            values.insert(slot);
        } else {
            values.remove(&slot);
        }
        let value = Value::IntArray(values.into_iter().map(i32::from).collect());
        Ok(vec![if self.disabled_present {
            Operation::Replace {
                path: format!("{parent}/disabled_slots"),
                value,
            }
        } else {
            Operation::InsertField {
                path: parent.into(),
                name: "disabled_slots".into(),
                value,
            }
        }])
    }
    /// 同一文档内跨背包移动或交换，源与目标库存的操作一起提交为一个撤销步骤。
    pub fn transfer_to(
        &self,
        target: &Self,
        from: i16,
        to: i16,
    ) -> Result<Vec<Operation>, WorldError> {
        if !Arc::ptr_eq(&self.revision, &target.revision) {
            return Err(error("来源和目标不是同一个文档世代"));
        }
        if self.spec.path == target.spec.path {
            return self.swap(from, to);
        }
        let stack = self.copy(from)?;
        let mut operations = target.paste(to, &stack)?;
        operations.extend(if target.items.contains_key(&to) {
            self.paste(from, &target.copy(to)?)?
        } else {
            self.remove(from)?
        });
        Ok(operations)
    }
    pub fn paste(&self, slot: i16, stack: &Stack) -> Result<Vec<Operation>, WorldError> {
        if self.spec.context.version != stack.context.version {
            return Err(error("物品跨版本转换尚未实现，请使用相同数据版本"));
        }
        let numeric = matches!(field(fields(&stack.value)?, "id")?, Some(Value::Short(_)));
        if (numeric || stack.context.version.is_none())
            && self.spec.context.root != stack.context.root
        {
            return Err(error("未知版本或数字物品只能在原世界使用"));
        }
        if self.spec.context.root != stack.context.root
            && stack
                .context
                .version
                .is_some_and(|version| crate::hanging::is_map_item(&stack.value, version))
        {
            return Err(error(
                "填充地图引用原世界的地图文件，不能直接粘贴到其他世界",
            ));
        }
        self.place(slot, stack.value.clone())
    }
}
fn validate_id(id: &str) -> Result<(), WorldError> {
    let Some((namespace, path)) = id.split_once(':') else {
        return Err(error("物品 ID 必须包含命名空间"));
    };
    let valid = |part: &str, is_path: bool| {
        !part.is_empty()
            && part.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || matches!(b, b'_' | b'-' | b'.')
                    || is_path && b == b'/'
            })
    };
    if !valid(namespace, false) || !valid(path, true) {
        return Err(error("物品 ID 含无效资源位置字符"));
    }
    Ok(())
}
fn new_stack(id: &str, count: i32, version: Option<i32>) -> Result<Value, WorldError> {
    validate_id(id)?;
    let mut data = vec![Field {
        name: "id".into(),
        value: Value::String(id.into()),
    }];
    match version {
        Some(1343 | 3465) => {
            data.push(Field {
                name: "Count".into(),
                value: Value::Byte(
                    i8::try_from(count).map_err(|_| error("旧版数量超出 Byte 范围"))?,
                ),
            });
            if version == Some(1343) {
                data.push(Field {
                    name: "Damage".into(),
                    value: Value::Short(0),
                });
            }
        }
        Some(3955) => data.push(Field {
            name: "count".into(),
            value: Value::Int(count),
        }),
        _ => {
            return Err(error(
                "当前数据版本的新物品编码尚未核实；已有物品可按实际字段编辑",
            ))
        }
    }
    Ok(Value::Compound(data))
}

/// 修改完整物品的基本字段；供预设与现有槽位共用，保留完整附加数据。
fn edit_fields(
    mut value: Value,
    version: Option<i32>,
    id: &str,
    count: i32,
    damage: Option<i16>,
) -> Result<Value, WorldError> {
    let id = id.trim();
    if count <= 0 {
        return Err(error("物品数量必须大于零；清空槽位请使用删除"));
    }
    let Value::Compound(data) = &mut value else {
        return Err(error("物品必须为 Compound"));
    };
    let id = match field(data, "id")? {
        Some(Value::Short(_)) => Value::Short(
            id.trim()
                .parse()
                .map_err(|_| error("旧数字物品 ID 必须为 Short 整数"))?,
        ),
        Some(Value::String(_)) => {
            validate_id(id)?;
            Value::String(id.into())
        }
        _ => return Err(error("物品 id 类型无效")),
    };
    let count_field = match (field(data, "Count")?, field(data, "count")?) {
        (Some(Value::Byte(_)), None) => (
            "Count",
            Value::Byte(i8::try_from(count).map_err(|_| error("旧版数量超出 Byte 范围"))?),
        ),
        (None, Some(Value::Int(_))) => ("count", Value::Int(count)),
        _ => return Err(error("物品数量字段无效")),
    };
    set(data, "id", id);
    set(data, count_field.0, count_field.1);
    if let Some(damage) = damage {
        if !matches!(field(data, "Damage")?, Some(Value::Short(_)))
            && !(field(data, "Damage")?.is_none() && version == Some(1343))
        {
            return Err(error("该物品没有旧版 Damage 字段；组件请在 NBT 中编辑"));
        }
        set(data, "Damage", Value::Short(damage));
    }
    Ok(value)
}
impl Stack {
    /// 为准确版本的填充地图指定已存在的世界地图 ID，保留预设的其他字段。
    pub fn with_map_id(&self, id: i32) -> Result<Self, WorldError> {
        let version = self
            .context
            .version
            .ok_or_else(|| error("地图物品缺少数据版本"))?;
        if id < 0 || !crate::hanging::is_map_item(&self.value, version) {
            return Err(error("地图 ID 为负数或当前物品不是填充地图"));
        }
        validate_version(&self.value, Some(version))?;
        let mut value = self.value.clone();
        let Value::Compound(data) = &mut value else {
            unreachable!("物品版本验证要求 Compound");
        };
        match version {
            1343 => set(
                data,
                "Damage",
                Value::Short(
                    i16::try_from(id).map_err(|_| error("1.12.2 地图 ID 超出非负 Short 范围"))?,
                ),
            ),
            3465 | 3955 => {
                let (container, field_name) = if version == 3465 {
                    ("tag", "map")
                } else {
                    ("components", "minecraft:map_id")
                };
                let mut contents = match field(data, container)? {
                    Some(Value::Compound(contents)) => contents.clone(),
                    None => Vec::new(),
                    _ => return Err(error(format!("地图物品 {container} 必须为 Compound"))),
                };
                match field(&contents, field_name)? {
                    Some(Value::Int(_)) | None => (),
                    _ => {
                        return Err(error(format!(
                            "地图物品 {container}.{field_name} 必须为 Int"
                        )))
                    }
                }
                set(&mut contents, field_name, Value::Int(id));
                set(data, container, Value::Compound(contents));
            }
            _ => {
                return Err(error(format!(
                    "尚未核对 DataVersion {version} 的地图物品编码"
                )))
            }
        }
        Ok(Self {
            value,
            context: self.context.clone(),
        })
    }
    /// 完整预设只改用户填写的字段；并不修改原预设或文档。
    pub fn with_fields(
        &self,
        id: &str,
        count: i32,
        damage: Option<i16>,
    ) -> Result<Self, WorldError> {
        Ok(Self {
            value: edit_fields(self.value.clone(), self.context.version, id, count, damage)?,
            context: self.context.clone(),
        })
    }
}
