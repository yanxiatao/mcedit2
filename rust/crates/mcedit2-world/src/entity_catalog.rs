//! 原始 Java 实体 NBT 读取与来源快照；供场景、Inspector 和独立编辑事务使用。
//! 现代独立 entities Region 与旧版 Level 内嵌实体分别沿用实际保存布局。
use crate::block_lookup::read_world_chunk_bytes;
use crate::sections::parse_root;
use crate::{RegionIndex, WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
/// 保存格式中的掉落物实体类型；物品模型继续由 Item 载荷及资源注册表解释。
pub fn is_dropped(id: &str) -> bool {
    matches!(id, "Item" | "minecraft:item")
}
pub fn is_experience_orb(id: &str) -> bool {
    matches!(
        id,
        "XPOrb" | "minecraft:xp_orb" | "minecraft:experience_orb"
    )
}
fn fail(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn sha(x: &[u8]) -> String {
    Sha256::digest(x)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Debug, Clone)]
pub struct SourceNbt {
    bytes: std::sync::Arc<Vec<u8>>,
    payload: std::ops::Range<usize>,
    fields: std::sync::Arc<Result<BTreeMap<String, SourceField>, String>>,
}
#[derive(Debug)]
struct SourceField {
    kind: u8,
    payload: std::ops::Range<usize>,
}
impl PartialEq for SourceNbt {
    fn eq(&self, other: &Self) -> bool {
        if std::sync::Arc::ptr_eq(&self.bytes, &other.bytes) && self.payload == other.payload {
            return true;
        }
        self.bytes.get(self.payload.clone()) == other.bytes.get(other.payload.clone())
    }
}
impl SourceNbt {
    fn new(
        bytes: std::sync::Arc<Vec<u8>>,
        payload: std::ops::Range<usize>,
        entry: &crate::sections::Tag<'_>,
    ) -> Self {
        let fields = (|| {
            if payload.end > bytes.len() || payload.len() != entry.raw.len() {
                return Err("源 NBT 快照范围失配".into());
            }
            let mut fields = BTreeMap::new();
            for (name, tag) in &entry.fields {
                let relative = (tag.raw.as_ptr() as usize)
                    .checked_sub(entry.raw.as_ptr() as usize)
                    .ok_or("源字段不属于原始 Compound")?;
                let start = payload
                    .start
                    .checked_add(relative)
                    .ok_or("源字段偏移溢出")?;
                let end = start
                    .checked_add(tag.raw.len())
                    .filter(|end| *end <= payload.end)
                    .ok_or("源字段范围溢出")?;
                if fields
                    .insert(
                        name.clone(),
                        SourceField {
                            kind: tag.kind,
                            payload: start..end,
                        },
                    )
                    .is_some()
                {
                    return Err(format!("源外观存在重复字段 {name}"));
                }
            }
            Ok(fields)
        })();
        Self {
            bytes,
            payload,
            fields: std::sync::Arc::new(fields),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct EntityVisual {
    /// 当前外观所属实际载荷的 DataVersion；不存在时保持未知。
    pub source_version: Option<i32>,
    /// 同代源 Compound 的有界快照；扩展按字段延迟解析，不受原版字段清单限制。
    pub source_nbt: Option<SourceNbt>,
    /// 原始偏航 / 俯仰角（度）；源字段缺失时保持 None，不伪造朝向。
    pub rotation: Option<[f32; 2]>,
    /// 保留渲染相关 NBT 的实际类型和内容；不把实体标记误作已经绘制的模型。
    pub appearance: BTreeMap<String, crate::nbt_document::Value>,
}
impl EntityVisual {
    /// 原生 Entity.load 后的运动向量；模型与装备共用实际载入语义，不推进运动。
    pub fn loaded_motion(&self) -> Result<[f64; 3], String> {
        use crate::nbt_document::Value;
        if !matches!(self.source_version, Some(1343 | 3465 | 3955)) {
            return Err(format!("尚未核实 {:?} 的实体运动载入", self.source_version));
        }
        let Some(value) = self.source_field_optional("Motion")? else {
            return Ok([0.; 3]);
        };
        let Value::List(list) = value else {
            return Err("实体 Motion 必须为三项 Double 列表".into());
        };
        if list.element_type != 6 || list.items.len() != 3 {
            return Err("实体 Motion 必须为三项 Double 列表".into());
        }
        let mut velocity = [0.; 3];
        for (out, value) in velocity.iter_mut().zip(list.items) {
            let Value::Double(value) = value else {
                return Err("实体 Motion 必须为 Double".into());
            };
            if !value.is_finite() {
                return Err("实体 Motion 包含非有限 Double".into());
            }
            // 三版准确客户端均在读取后将绝对值大于 10 的分量置零；源载荷不改写。
            *out = if value.abs() > 10. { 0. } else { value };
        }
        Ok(velocity)
    }
    /// 玩家存档的实际游戏模式决定原生身体 / 附加层可见性，不借装备或能力猜测。
    pub fn player_spectator(&self) -> Result<bool, String> {
        if !matches!(self.source_version, Some(1343 | 3465 | 3955)) {
            return Err(format!(
                "尚未核实版本 {:?} 的玩家游戏模式",
                self.source_version
            ));
        }
        match self.source_field_optional("playerGameType")? {
            None | Some(crate::nbt_document::Value::Int(0..=2)) => Ok(false),
            Some(crate::nbt_document::Value::Int(3)) => Ok(true),
            Some(crate::nbt_document::Value::Int(value)) => {
                Err(format!("未定义的玩家 playerGameType={value}"))
            }
            _ => Err("玩家 playerGameType 必须为 Int".into()),
        }
    }

    /// 驯服动物的实际物主编码；项圈与身体共用，不从外观变种猜测驯服状态。
    pub fn tamed(&self) -> Result<bool, String> {
        match self.source_version {
            Some(1343) => match self.source_field_optional("OwnerUUID")? {
                None if self.source_field_optional("Owner")?.is_none() => Ok(false),
                Some(crate::nbt_document::Value::String(value)) if value.is_empty() => Ok(false),
                Some(crate::nbt_document::Value::String(value)) => {
                    crate::player_catalog::uuid_text(&value).ok_or("旧驯服实体 OwnerUUID 无效")?;
                    Ok(true)
                }
                _ => Err("旧驯服实体缺少可解析的 OwnerUUID，不能推测旧名称对应玩家".into()),
            },
            Some(3465 | 3955) => match self.source_field_optional("Owner")? {
                None => Ok(false),
                Some(crate::nbt_document::Value::IntArray(words)) if words.len() == 4 => Ok(true),
                _ => Err("驯服实体 Owner 必须为四项 UUID IntArray".into()),
            },
            version => Err(format!("未核实版本 {version:?} 的驯服物主编码")),
        }
    }
    pub fn parse_block_entity(compound: &[u8], version: Option<i32>) -> Result<Self, String> {
        let mut raw = Vec::with_capacity(compound.len() + 3);
        raw.extend_from_slice(&[10, 0, 0]);
        raw.extend_from_slice(compound);
        let root = parse_root(&raw).map_err(|e| e.to_string())?;
        Self::read_block_entity(&root, version)
    }
    pub fn source_field(&self, name: &str) -> Result<crate::nbt_document::Value, String> {
        self.source_field_optional(name)?
            .ok_or_else(|| format!("源外观缺少字段 {name}"))
    }
    /// 只将实际不存在的字段表示为 None；损坏、类型和快照范围错误继续返回 Err。
    pub fn source_field_optional(
        &self,
        name: &str,
    ) -> Result<Option<crate::nbt_document::Value>, String> {
        if let Some(value) = self.appearance.get(name) {
            return Ok(Some(value.clone()));
        }
        let source = self
            .source_nbt
            .as_ref()
            .ok_or("源外观快照未保留或超过 1 MiB 预算，无法读取扩展 NBT 字段")?;
        let fields = source.fields.as_ref().as_ref().map_err(Clone::clone)?;
        let Some(field) = fields.get(name) else {
            return Ok(None);
        };
        let payload = source
            .bytes
            .get(field.payload.clone())
            .ok_or("源外观字段范围无效")?;
        // 只解析所取字段，不为每次绑定重新遍历库存和乘客等无关 Compound。
        let mut raw = Vec::with_capacity(payload.len() + 7);
        raw.extend_from_slice(&[10, 0, 0, field.kind, 0, 0]);
        raw.extend_from_slice(payload);
        raw.push(0);
        let root = parse_root(&raw).map_err(|e| e.to_string())?;
        let (_, field) = root.fields.first().ok_or("源外观字段封装缺少成员")?;
        crate::nbt_document::value(field)
            .map(Some)
            .map_err(|e| e.to_string())
    }
    pub(crate) fn read(entry: &crate::sections::Tag<'_>) -> Result<Self, String> {
        let source = Some(SourceNbt::new(
            std::sync::Arc::new(entry.raw.to_vec()),
            0..entry.raw.len(),
            entry,
        ));
        Self::decode(entry, source, true)
    }
    pub(crate) fn read_block_entity(
        entry: &crate::sections::Tag<'_>,
        version: Option<i32>,
    ) -> Result<Self, String> {
        if entry.kind != 10 {
            return Err("方块实体外观必须为 Compound".into());
        }
        let source = Some(SourceNbt::new(
            std::sync::Arc::new(entry.raw.to_vec()),
            0..entry.raw.len(),
            entry,
        ));
        let mut visual = Self::decode(entry, source, false)?;
        visual.source_version = version;
        Ok(visual)
    }
    fn read_shared(
        entry: &crate::sections::Tag<'_>,
        bytes: &std::sync::Arc<Vec<u8>>,
        original: &[u8],
    ) -> Result<Self, String> {
        let start = (entry.raw.as_ptr() as usize)
            .checked_sub(original.as_ptr() as usize)
            .ok_or("实体外观不属于共享源载荷")?;
        let end = start
            .checked_add(entry.raw.len())
            .filter(|end| *end <= bytes.len())
            .ok_or("共享实体源范围无效")?;
        let source = Some(SourceNbt::new(
            std::sync::Arc::clone(bytes),
            start..end,
            entry,
        ));
        Self::decode(entry, source, true)
    }
    fn decode(
        entry: &crate::sections::Tag<'_>,
        source_nbt: Option<SourceNbt>,
        entity_rotation: bool,
    ) -> Result<Self, String> {
        let rotation = if entity_rotation {
            entry
                .field("Rotation")
                .map(|tag| {
                    if tag.kind != 9
                        || tag.items.len() != 2
                        || tag.items.iter().any(|t| t.kind != 5 || t.raw.len() != 4)
                    {
                        return Err("Rotation 必须为两项 Float 列表".to_string());
                    }
                    let angles = std::array::from_fn(|i| {
                        f32::from_be_bytes(tag.items[i].raw.try_into().unwrap())
                    });
                    if !angles.into_iter().all(f32::is_finite) {
                        return Err("Rotation 含非有限角度".into());
                    }
                    Ok(angles)
                })
                .transpose()?
        } else {
            None
        };
        let mut appearance = BTreeMap::new();
        for name in [
            "Pose",
            "Age",
            "IsBaby",
            "IsImmuneToZombification",
            "NoAI",
            "Small",
            "NoBasePlate",
            "ShowArms",
            "Invisible",
            "Marker",
            "Color",
            "Sheared",
            "sheared",
            "Variant",
            "variant",
            "Type",
            "VillagerData",
            "Profession",
            "RabbitType",
            "CatType",
            "Pumpkin",
            "HasEgg",
            "HasNectar",
            "HasStung",
            "MainGene",
            "HiddenGene",
            "AttackTick",
            "StunTick",
            "RoarTick",
            "Invul",
            "BatFlags",
            "DarkTicksRemaining",
            "OnGround",
            "ShoulderEntityLeft",
            "ShoulderEntityRight",
            "playerGameType",
            "ConversionTime",
            "TicksFrozen",
            "SpellTicks",
            "PuffState",
            "Size",
            "carried",
            "carriedData",
            "carriedBlockState",
            "AttachFace",
            "Peek",
            "Saddle",
            "attributes",
            "CustomName",
            "CustomNameVisible",
            "ArmorItems",
            "HandItems",
            "body_armor_item",
            "ArmorItem",
            "DecorItem",
            "SaddleItem",
            "ChestedHorse",
            "EatingHaystack",
            "Owner",
            "OwnerUUID",
            "Sitting",
            "Sleeping",
            "Crouching",
            "HasLeftHorn",
            "HasRightHorn",
            "AngerTime",
            "Angry",
            "CollarColor",
            "Health",
            "Attributes",
            "LeftHanded",
            "Inventory",
            "SelectedItemSlot",
            "LastDeathLocation",
            "FallFlying",
            "Motion",
            "Item",
            "Facing",
            "ItemRotation",
            "TileX",
            "TileY",
            "TileZ",
            "Command",
        ] {
            if let Some(tag) = entry.field(name) {
                let value = crate::nbt_document::value(tag).map_err(|e| e.to_string())?;
                fn finite(value: &crate::nbt_document::Value) -> bool {
                    use crate::nbt_document::Value;
                    match value {
                        Value::Float(v) => v.is_finite(),
                        Value::Double(v) => v.is_finite(),
                        Value::List(list) => list.items.iter().all(finite),
                        Value::Compound(fields) => fields.iter().all(|f| finite(&f.value)),
                        _ => true,
                    }
                }
                // 完整物品的未知附加 NBT 原样保留；真正参与显示的数值由物品解释层校验。
                if !matches!(
                    name,
                    "Item"
                        | "ArmorItems"
                        | "HandItems"
                        | "body_armor_item"
                        | "Inventory"
                        | "ArmorItem"
                        | "DecorItem"
                        | "SaddleItem"
                ) && !finite(&value)
                {
                    return Err(format!("外观字段 {name} 含非有限数"));
                }
                appearance.insert(name.into(), value);
            }
        }
        Ok(Self {
            source_version: None,
            source_nbt,
            rotation,
            appearance,
        })
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct OriginalEntity {
    pub id: Option<String>,
    /// 在原始实体树中的位置，包含乘客和旧版 Riding 载具。
    pub path: String,
    pub pos: Option<[f64; 3]>,
    pub fields: usize,
    pub bytes: usize,
    pub sha256: String,
    /// 外观损坏不会抹掉有效的位置；使用方须明确处理此诊断。
    pub visual: Result<EntityVisual, String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct EntityCatalog {
    pub source: String,
    pub chunk: (i32, i32),
    pub source_sha256: String,
    pub entities: Vec<OriginalEntity>,
}
pub(crate) enum Storage {
    Embedded,
    Separate(Option<(String, Vec<u8>)>),
}
pub(crate) fn storage(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
) -> Result<Storage, WorldError> {
    let dimension = world
        .dimensions
        .iter()
        .find(|v| v.name == dim)
        .ok_or_else(|| fail("Unknown original dimension"))?;
    if dimension.extension.contains("c.x.z.dat") {
        return Ok(Storage::Embedded);
    }
    if let Some(parent) = dimension.region_dir.parent() {
        let name = parent.join("entities").join(format!(
            "r.{}.{}.mca",
            cx.div_euclid(32),
            cz.div_euclid(32)
        ));
        if name.exists() {
            if fs::symlink_metadata(&name)?.file_type().is_symlink() {
                return Err(fail("Entity Region symlink refused"));
            }
            let real = fs::canonicalize(&name)?;
            if !real.starts_with(fs::canonicalize(&world.root)?) {
                return Err(fail("Entity Region escapes original world"));
            }
            let index = RegionIndex::open(&real)?;
            return Ok(Storage::Separate(
                index
                    .read_chunk_at(cx, cz)?
                    .map(|raw| (format!("entity-region:{}", real.display()), raw)),
            ));
        }
    }
    Ok(Storage::Embedded)
}
pub(crate) fn source(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
) -> Result<Option<(String, Vec<u8>)>, WorldError> {
    match storage(world, dim, cx, cz)? {
        Storage::Separate(Some(data)) => Ok(Some(data)),
        Storage::Separate(None) => {
            let Some(raw) = read_world_chunk_bytes(world, dim, cx, cz)? else {
                return Ok(None);
            };
            let catalog = parse(&raw, "legacy-main-chunk", cx, cz)?;
            if catalog.entities.is_empty() {
                Ok(None)
            } else {
                Ok(Some(("legacy-main-chunk".into(), raw)))
            }
        }
        Storage::Embedded => read_world_chunk_bytes(world, dim, cx, cz)
            .map(|data| data.map(|raw| ("legacy-main-chunk".into(), raw))),
    }
}
/// 历史来源保留原始 entities Region 及外置流，不用当前文件冒充旧载荷。
pub(crate) fn storage_from(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
    origin: Option<&crate::history_source::Archive>,
) -> Result<Storage, WorldError> {
    let Some(origin) = origin else {
        return storage(world, dim, cx, cz);
    };
    if origin.world_root() != world.root.as_path() {
        return Err(fail("实体历史来源属于其他世界"));
    }
    let dimension = world
        .dimensions
        .iter()
        .find(|entry| entry.name == dim)
        .ok_or_else(|| fail("实体来源维度不存在"))?;
    if dimension.extension.contains("c.x.z.dat") {
        return Ok(Storage::Embedded);
    }
    let Some(parent) = dimension.region_dir.parent() else {
        return Ok(Storage::Embedded);
    };
    use crate::entity_editing::IdentitySource;
    let directory = parent.join("entities");
    if !origin.is_directory(&directory)? {
        return Ok(Storage::Embedded);
    }
    let path = directory.join(format!("r.{}.{}.mca", cx.div_euclid(32), cz.div_euclid(32)));
    let Some(image) = origin.read_reserved(&path, 256 * 1024 * 1024, |_| Ok(()))? else {
        return Ok(Storage::Embedded);
    };
    let slots = crate::region_plan::parse_slots(&image)?;
    let index = (cz.rem_euclid(32) * 32 + cx.rem_euclid(32)) as usize;
    let Some(slot) = slots[index] else {
        return Ok(Storage::Separate(None));
    };
    let raw = if let Some(method) = crate::region_external::is_external(&image, slot)? {
        let side = crate::region_external::sidecar_path(&path, cx, cz)?;
        let bytes = origin
            .read_reserved(&side, 32 * 1024 * 1024, |_| Ok(()))?
            .ok_or_else(|| fail("实体历史 Region 指向的 MCC 文件缺失"))?;
        crate::region_external::decode(method, &bytes)?.1
    } else {
        crate::region_plan::original_nbt(&image, slot)?.2
    };
    Ok(Storage::Separate(Some((
        format!("entity-region:{}", path.display()),
        raw,
    ))))
}
pub(crate) fn source_from(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
    origin: Option<&crate::history_source::Archive>,
) -> Result<Option<(String, Vec<u8>)>, WorldError> {
    let separate = match storage_from(world, dim, cx, cz, origin)? {
        Storage::Separate(Some(data)) => return Ok(Some(data)),
        Storage::Separate(None) => true,
        Storage::Embedded => false,
    };
    let raw = crate::block_lookup::ReadOnlyRegionCache::for_source(world, dim, origin)?
        .read_chunk_bytes(cx, cz)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    if separate
        && parse(&raw, "legacy-main-chunk", cx, cz)?
            .entities
            .is_empty()
    {
        return Ok(None);
    }
    Ok(Some(("legacy-main-chunk".into(), raw)))
}
pub(crate) fn parse(
    original: &[u8],
    label: &str,
    cx: i32,
    cz: i32,
) -> Result<EntityCatalog, WorldError> {
    let root = parse_root(original).map_err(|e| fail(format!("Entity NBT invalid: {e}")))?;
    let version = root
        .field("DataVersion")
        .map(|tag| tag.i32().map_err(|e| fail(e.to_string())))
        .transpose()?;
    let modern = label.starts_with("entity-region:");
    let container = if modern {
        &root
    } else {
        root.field("Level").unwrap_or(&root)
    };
    if container.kind != 10 {
        return Err(fail("Entity storage not a Compound"));
    }
    if modern {
        let position = container
            .require("Position", 11)
            .map_err(|e| fail(e.to_string()))?;
        if position.raw.len() != 12
            || i32::from_be_bytes(position.raw[..4].try_into().unwrap()) != 2
            || i32::from_be_bytes(position.raw[4..8].try_into().unwrap()) != cx
            || i32::from_be_bytes(position.raw[8..12].try_into().unwrap()) != cz
        {
            return Err(fail("Entity file Position does not match its Region slot"));
        }
    }
    let mut entities = Vec::new();
    if let Some(list) = container.field("Entities") {
        if list.kind != 9 {
            return Err(fail(
                "Original entity List has incorrect NBT schema or count",
            ));
        }
        let shared = std::sync::Arc::new(original.to_vec());
        for (index, entry) in list.items.iter().enumerate() {
            collect_entity(
                entry,
                format!("Entities/{index}"),
                &mut entities,
                Some((&shared, original)),
                version,
            )?;
        }
    }
    Ok(EntityCatalog {
        source: label.into(),
        chunk: (cx, cz),
        source_sha256: sha(original),
        entities,
    })
}
fn collect_entity(
    entry: &crate::sections::Tag<'_>,
    path: String,
    out: &mut Vec<OriginalEntity>,
    shared: Option<(&std::sync::Arc<Vec<u8>>, &[u8])>,
    version: Option<i32>,
) -> Result<(), WorldError> {
    if entry.kind != 10 {
        return Err(fail("实体树成员不是 Compound"));
    }
    let id = entry
        .field("id")
        .map(|v| v.string().map_err(|e| fail(e.to_string())))
        .transpose()?;
    let pos = if let Some(v) = entry.field("Pos") {
        if v.kind != 9
            || v.items.len() != 3
            || v.items.iter().any(|x| x.kind != 6 || x.raw.len() != 8)
        {
            return Err(fail(format!("实体 {path} 的 Pos 不是三项 Double 列表")));
        }
        let xyz = std::array::from_fn(|i| f64::from_be_bytes(v.items[i].raw.try_into().unwrap()));
        if !xyz.into_iter().all(f64::is_finite) {
            return Err(fail(format!("实体 {path} 的坐标不是有限数")));
        }
        Some(xyz)
    } else {
        None
    };
    out.push(OriginalEntity {
        id,
        path: path.clone(),
        pos,
        fields: entry.fields.len(),
        bytes: entry.raw.len(),
        sha256: sha(entry.raw),
        visual: match shared {
            Some((bytes, original)) => EntityVisual::read_shared(entry, bytes, original),
            None => EntityVisual::read(entry),
        }
        .map(|mut visual| {
            visual.source_version = version;
            visual
        }),
    });
    if let Some(list) = entry.field("Passengers") {
        if list.kind != 9 {
            return Err(fail(format!("实体 {path} 的 Passengers 不是列表")));
        }
        for (i, passenger) in list.items.iter().enumerate() {
            collect_entity(
                passenger,
                format!("{path}/Passengers/{i}"),
                out,
                shared,
                version,
            )?;
        }
    }
    if let Some(vehicle) = entry.field("Riding") {
        collect_entity(vehicle, format!("{path}/Riding"), out, shared, version)?;
    }
    Ok(())
}

pub fn inspect(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
) -> Result<EntityCatalog, WorldError> {
    let (label, raw) =
        source(world, dim, cx, cz)?.ok_or_else(|| fail("Original entity chunk not present"))?;
    parse(&raw, &label, cx, cz)
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitySnapshot {
    version: u32,
    root: String,
    dimension: String,
    chunk: [i32; 2],
    source: String,
    digest: String,
    original_nbt: Vec<u8>,
}
pub fn export(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
    path: &Path,
) -> Result<usize, WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("mcedit2entities") {
        return Err(fail("Entity snapshot requires .mcedit2entities"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(fail("Entity snapshot must be outside Minecraft world"));
    }
    let (label, raw) =
        source(world, dim, cx, cz)?.ok_or_else(|| fail("Entity source disappeared"))?;
    if raw.len() > 8 * 1024 * 1024 {
        return Err(fail("Entity source >8MiB"));
    }
    parse(&raw, &label, cx, cz)?;
    let data = EntitySnapshot {
        version: 1,
        root: world.root.to_string_lossy().into(),
        dimension: dim.into(),
        chunk: [cx, cz],
        source: label,
        digest: sha(&raw),
        original_nbt: raw,
    };
    let bytes = serde_json::to_vec(&data)
        .map_err(|e| fail(format!("Entity snapshot JSON invalid: {e}")))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(fail("Entity snapshot >32MiB"));
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let status = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = status {
        let _ = fs::remove_file(path);
        return Err(e.into());
    }
    Ok(bytes.len())
}
pub fn verify(world: &WorldSummary, dim: &str, path: &Path) -> Result<usize, WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("mcedit2entities") {
        return Err(fail("Unknown entity snapshot extension"));
    }
    let stat = fs::symlink_metadata(path)?;
    if !stat.file_type().is_file() || stat.file_type().is_symlink() || stat.len() > 32 * 1024 * 1024
    {
        return Err(fail("Entity snapshot is not a safe regular file"));
    }
    let snap: EntitySnapshot = serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| fail(format!("Entity snapshot JSON invalid: {e}")))?;
    if snap.version != 1
        || snap.root != world.root.to_string_lossy()
        || snap.dimension != dim
        || snap.original_nbt.len() > 8 * 1024 * 1024
        || sha(&snap.original_nbt) != snap.digest
    {
        return Err(fail("Entity snapshot checksum/world identity invalid"));
    }
    let (origin, actual) = source(world, dim, snap.chunk[0], snap.chunk[1])?
        .ok_or_else(|| fail("Original entity data disappeared"))?;
    if origin != snap.source || actual != snap.original_nbt {
        return Err(fail(
            "Entity Region differs from immutable original snapshot",
        ));
    }
    parse(&actual, &origin, snap.chunk[0], snap.chunk[1])?;
    Ok(actual.len())
}
/// 解析结构中的实体根树，不创建磁盘来源或分配 UUID。
pub fn parse_preview(compound: &[u8]) -> Result<Vec<OriginalEntity>, String> {
    let mut bytes = Vec::with_capacity(compound.len() + 3);
    bytes.extend([10, 0, 0]);
    bytes.extend_from_slice(compound);
    let root = parse_root(&bytes).map_err(|e| format!("待放置实体 NBT 无效：{e}"))?;
    let mut entities = Vec::new();
    let shared = std::sync::Arc::new(bytes.clone());
    collect_entity(
        &root,
        "preview".into(),
        &mut entities,
        Some((&shared, &bytes)),
        None,
    )
    .map_err(|e| e.to_string())?;
    Ok(entities)
}
