//! 地图颜色转换与候选计划；实际写入由统一事务负责。
use crate::map_catalog::Catalog;
use sha2::{Digest, Sha256};
/// 由世界地图物品生成单个展示框原稿；实体身份和目标区块预检由现有结构粘贴负责。
pub fn frame_structure(
    session: &crate::editing::Session,
    stack: &crate::inventory::Stack,
    support: [i32; 3],
    facing: u8,
    budget: crate::editing::EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<crate::clipboard::BlockClipboard, crate::WorldError> {
    use crate::{
        nbt_document::{Field, List, Value},
        WorldError,
    };
    let invalid = |message: &str| WorldError::Invalid(message.into());
    let world = session.original_world();
    let version = world
        .data_version
        .ok_or_else(|| invalid("展示框来源没有数据版本"))?;
    if stack.context.root != world.root || stack.context.version != Some(version) {
        return Err(invalid("地图物品和展示框必须属于同一个世界与数据版本"));
    }
    if !crate::hanging::is_map_item(&stack.value, version) {
        return Err(invalid("场景只接受填充地图物品"));
    }
    let encoded =
        crate::hanging::encode_facing(facing, version, false).map_err(WorldError::Invalid)?;
    let normal = crate::hanging::DIRECTIONS[usize::from(facing)];
    let mut tile = [0; 3];
    for axis in 0..3 {
        tile[axis] = support[axis]
            .checked_add(normal[axis])
            .ok_or_else(|| invalid("展示框坐标溢出"))?;
    }
    let mut clip = session.capture_block_states_with_budget_cancellable(
        crate::editing::Bounds {
            minimum: support,
            maximum: support,
        },
        budget,
        &mut cancelled,
    )?;
    if cancelled() {
        return Err(invalid("地图展示框创建已取消"));
    }
    let position = crate::hanging::anchor_center(tile, facing);
    let compound = Value::Compound(vec![
        Field {
            name: "id".into(),
            value: Value::String("minecraft:item_frame".into()),
        },
        Field {
            name: "Pos".into(),
            value: Value::List(List {
                element_type: 6,
                items: position.into_iter().map(Value::Double).collect(),
            }),
        },
        Field {
            name: "Rotation".into(),
            value: Value::List(List {
                element_type: 5,
                items: crate::hanging::angles(facing)
                    .into_iter()
                    .map(Value::Float)
                    .collect(),
            }),
        },
        Field {
            name: "TileX".into(),
            value: Value::Int(tile[0]),
        },
        Field {
            name: "TileY".into(),
            value: Value::Int(tile[1]),
        },
        Field {
            name: "TileZ".into(),
            value: Value::Int(tile[2]),
        },
        Field {
            name: "Facing".into(),
            value: Value::Byte(encoded),
        },
        Field {
            name: "Item".into(),
            value: stack.value.clone(),
        },
        Field {
            name: "ItemRotation".into(),
            value: Value::Byte(0),
        },
        Field {
            name: "ItemDropChance".into(),
            value: Value::Float(1.),
        },
    ])
    .payload()?;
    clip.source_bound = false;
    clip.source_min = tile;
    // 网格保留真实来源状态的编码，但不声明或修改任何方块。
    clip.mask = Some(vec![false]);
    clip.entity_dimension = matches!(version, 3465 | 3955).then(|| session.dimension().into());
    clip.entities.push(crate::clipboard::OpaqueEntity {
        position: std::array::from_fn(|axis| position[axis] - f64::from(tile[axis])),
        block_position: Some([0; 3]),
        compound,
    });
    clip.validate_with_budget(world, session.dimension(), budget)?;
    Ok(clip)
}
#[derive(Debug, Clone)]
pub struct Tile {
    pub column: u32,
    pub row: u32,
    pub colors: Vec<i8>,
}
#[derive(Debug, Clone)]
pub struct Mosaic {
    pub data_version: i32,
    pub width: u32,
    pub height: u32,
    pub image_sha256: String,
    pub tiles: Vec<Tile>,
}
#[derive(Clone)]
struct ColorNode {
    code: u8,
    rgb: [u8; 3],
    axis: usize,
    left: Option<usize>,
    right: Option<usize>,
}
fn build_palette(
    mut entries: Vec<(u8, [u8; 3])>,
    depth: usize,
    nodes: &mut Vec<ColorNode>,
) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    let axis = depth % 3;
    entries.sort_by_key(|(code, rgb)| (rgb[axis], *code));
    let right = entries.split_off(entries.len() / 2 + 1);
    let (code, rgb) = entries.pop()?;
    let index = nodes.len();
    nodes.push(ColorNode {
        code,
        rgb,
        axis,
        left: None,
        right: None,
    });
    let left = build_palette(entries, depth + 1, nodes);
    let right = build_palette(right, depth + 1, nodes);
    nodes[index].left = left;
    nodes[index].right = right;
    Some(index)
}
fn nearest(nodes: &[ColorNode], index: Option<usize>, rgb: [u8; 3], best: &mut (i32, u8)) {
    let Some(i) = index else { return };
    let node = &nodes[i];
    let delta: [i32; 3] =
        std::array::from_fn(|axis| i32::from(rgb[axis]) - i32::from(node.rgb[axis]));
    let dist = delta.iter().map(|d| d * d).sum::<i32>();
    if (dist, node.code) < *best {
        *best = (dist, node.code);
    }
    let (near, far) = if delta[node.axis] < 0 {
        (node.left, node.right)
    } else {
        (node.right, node.left)
    };
    nearest(nodes, near, rgb, best);
    if delta[node.axis] * delta[node.axis] <= best.0 {
        nearest(nodes, far, rgb, best);
    }
}
/// 将真实 RGBA 像素按 128×128 分块并映射到同一版 MapColor；不足一块的边缘透明填充。
pub fn quantize_mosaic(
    version: Option<i32>,
    width: u32,
    height: u32,
    rgba: &[u8],
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Mosaic, String> {
    let version = version
        .filter(|v| matches!(*v, 1343 | 3465 | 3955))
        .ok_or("未知地图版本，拒绝猜测色表")?;
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or("图片尺寸溢出")?;
    if width == 0 || height == 0 || pixels.checked_mul(4) != Some(rgba.len()) {
        return Err("图片 RGBA 长度与尺寸不符".into());
    }
    let columns = width.checked_add(127).ok_or("图片宽度溢出")? / 128;
    let rows = height.checked_add(127).ok_or("图片高度溢出")? / 128;
    let count = (columns as usize)
        .checked_mul(rows as usize)
        .ok_or("地图块数溢出")?;
    let need = count
        .checked_mul(128 * 128)
        .and_then(|v| v.checked_add(rgba.len()))
        .ok_or("图片内存估算溢出")?;
    if need > memory_budget {
        return Err("图片和地图块超过本次内存预算".into());
    }
    let palette = (4u8..=247)
        .filter_map(|code| {
            crate::map_data::color_for_version(version, code)
                .map(|color| (code, [color[0], color[1], color[2]]))
        })
        .collect::<Vec<_>>();
    let mut nodes = Vec::with_capacity(palette.len());
    let root = build_palette(palette, 0, &mut nodes).ok_or("地图色表为空")?;
    let mut tiles = Vec::with_capacity(count);
    for row in 0..rows {
        for column in 0..columns {
            let mut colors = vec![0i8; 128 * 128];
            for y in 0..128u32 {
                if cancelled() {
                    return Err("地图颜色转换已取消".into());
                }
                for x in 0..128u32 {
                    let sx = column * 128 + x;
                    let sy = row * 128 + y;
                    if sx >= width || sy >= height {
                        continue;
                    }
                    let i = ((sy as usize) * (width as usize) + sx as usize) * 4;
                    let p = &rgba[i..i + 4];
                    if p[3] < 128 {
                        continue;
                    }
                    let mut best = (i32::MAX, u8::MAX);
                    nearest(&nodes, Some(root), [p[0], p[1], p[2]], &mut best);
                    colors[(y as usize) * 128 + x as usize] = best.1 as i8;
                }
            }
            tiles.push(Tile {
                column,
                row,
                colors,
            });
        }
    }
    Ok(Mosaic {
        data_version: version,
        width,
        height,
        image_sha256: format!("{:x}", Sha256::digest(rgba)),
        tiles,
    })
}
/// 按现有文件及 idcounts 的最后分配 ID 分配连续 ID；不会覆盖已有地图。
pub fn allocate(catalog: &Catalog, last: i32, count: usize) -> Result<Vec<i32>, String> {
    if last < -1 {
        return Err("idcounts 的地图 ID 无效".into());
    }
    let highest = catalog.entries.last().map_or(-1, |v| v.id).max(last);
    let mut result = Vec::with_capacity(count);
    let mut next = highest;
    for _ in 0..count {
        next = next.checked_add(1).ok_or("地图 ID 已耗尽")?;
        result.push(next);
    }
    Ok(result)
}

/// 新地图内容的领域证明；不允许调用方把任意字节冒充地图。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProof {
    pub id: i32,
    pub data_version: i32,
    pub dimension: String,
    pub colors: Vec<i8>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteProof {
    pub id: i32,
    pub data_version: i32,
    pub source_sha: String,
}
#[derive(Debug, Clone)]
pub enum Change {
    Create {
        proof: CreateProof,
        bytes: Vec<u8>,
    },
    Delete {
        proof: DeleteProof,
    },
    /// 新建后在保存前删除：保留真实 ID 分配，不生成地图文件。
    Discarded {
        proof: CreateProof,
    },
}
#[derive(Debug, Clone)]
pub enum Counter {
    Existing(crate::nbt_file::Candidate),
    New { last: i32, bytes: Vec<u8> },
}
#[derive(Debug, Clone)]
pub struct Candidate {
    pub root: std::path::PathBuf,
    pub data_version: i32,
    pub counter: Option<Counter>,
    pub changes: Vec<Change>,
    pub source_images: Vec<(std::path::PathBuf, String)>,
}
/// 当前世界已应用地图候选的不可变读取视图；撤销栈不参与显示。
/// 不重新准备写入证明，也不把候选写到临时文件。未改动的 ID 仍读取原文件。
#[derive(Clone, Debug)]
pub struct View {
    root: std::path::PathBuf,
    entries: std::sync::Arc<std::collections::BTreeMap<i32, ViewEntry>>,
}
#[derive(Debug)]
enum ViewEntry {
    Created {
        version: i32,
        bytes: std::sync::Arc<[u8]>,
        sha256: String,
        pending: bool,
    },
    Deleted {
        version: i32,
    },
}
impl PartialEq for View {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && std::sync::Arc::ptr_eq(&self.entries, &other.entries)
    }
}
impl Eq for View {}
impl View {
    /// 投影已经预检并接受的候选；按实际应用顺序保留每个 ID 的最终状态。
    pub fn from_candidates<'a>(
        root: &std::path::Path,
        candidates: impl IntoIterator<Item = &'a Candidate>,
    ) -> Self {
        let mut entries = std::collections::BTreeMap::new();
        for candidate in candidates.into_iter().filter(|c| c.root == root) {
            for change in &candidate.changes {
                let (id, entry) = match change {
                    Change::Create { proof, bytes } => (
                        proof.id,
                        ViewEntry::Created {
                            version: proof.data_version,
                            bytes: std::sync::Arc::from(bytes.as_slice()),
                            sha256: format!("{:x}", Sha256::digest(bytes)),
                            pending: true,
                        },
                    ),
                    Change::Delete { proof } => (
                        proof.id,
                        ViewEntry::Deleted {
                            version: proof.data_version,
                        },
                    ),
                    Change::Discarded { proof } => (
                        proof.id,
                        ViewEntry::Deleted {
                            version: proof.data_version,
                        },
                    ),
                };
                entries.insert(id, entry);
            }
        }
        Self {
            root: root.to_owned(),
            entries: std::sync::Arc::new(entries),
        }
    }
    /// 资料库显示有效内容，ID 分配仍使用原始磁盘清单及计数证明。
    pub fn catalog(&self, disk: &Catalog) -> Catalog {
        let mut entries = disk
            .entries
            .iter()
            .map(|e| (e.id, e.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        for (&id, entry) in self.entries.iter() {
            match entry {
                ViewEntry::Created { bytes, sha256, .. } => {
                    entries.insert(
                        id,
                        crate::map_catalog::Entry {
                            id,
                            path: self.root.join("data").join(format!("map_{id}.dat")),
                            sha256: sha256.clone(),
                            bytes: bytes.len() as u64,
                        },
                    );
                }
                ViewEntry::Deleted { .. } => {
                    entries.remove(&id);
                }
            }
        }
        Catalog {
            entries: entries.into_values().collect(),
            diagnostics: disk.diagnostics.clone(),
        }
    }
    pub fn contains_created(&self, id: i32) -> bool {
        matches!(
            self.entries.get(&id),
            Some(ViewEntry::Created { pending: true, .. })
        )
    }
    /// 地图历史投影包含保存点两侧的原始字节；已保存内容不标作待保存。
    pub(crate) fn from_files(
        root: &std::path::Path,
        version: i32,
        files: impl IntoIterator<Item = (i32, Option<std::sync::Arc<[u8]>>, bool)>,
    ) -> Self {
        let entries = files
            .into_iter()
            .map(|(id, bytes, pending)| {
                let entry = match bytes {
                    Some(bytes) => ViewEntry::Created {
                        version,
                        sha256: format!("{:x}", Sha256::digest(&bytes)),
                        bytes,
                        pending,
                    },
                    None => ViewEntry::Deleted { version },
                };
                (id, entry)
            })
            .collect();
        Self {
            root: root.to_owned(),
            entries: std::sync::Arc::new(entries),
        }
    }
    /// 来源世界 / 版本必须一致；候选删除明确报错，不回读已被遮蔽的磁盘文件。
    pub fn read(
        &self,
        root: &std::path::Path,
        id: i32,
        version: Option<i32>,
    ) -> Result<crate::map_data::MapImage, String> {
        if root != self.root {
            return Err("地图读取视图属于另一个世界".into());
        }
        match self.entries.get(&id) {
            Some(ViewEntry::Created {
                version: actual,
                bytes,
                ..
            }) => {
                if version != Some(*actual) {
                    return Err("地图候选与来源版本不同".into());
                }
                crate::map_data::from_bytes(
                    self.root.join("data").join(format!("map_{id}.dat")),
                    id,
                    version,
                    bytes,
                )
            }
            Some(ViewEntry::Deleted { version: actual }) => {
                if version != Some(*actual) {
                    return Err("地图删除候选与来源版本不同".into());
                }
                Err(format!("地图 #{id} 已在当前修订删除"))
            }
            None => crate::map_data::read(root, id, version),
        }
    }
}
fn raw_map(proof: &CreateProof) -> Result<Vec<u8>, String> {
    use crate::nbt_document::{Document, Field, List, Operation, Value};
    if proof.id < 0 || proof.colors.len() != 128 * 128 || proof.dimension.is_empty() {
        return Err("地图创建参数无效".into());
    }
    if proof
        .colors
        .iter()
        .any(|v| crate::map_data::color_for_version(proof.data_version, *v as u8).is_none())
    {
        return Err("地图包含未分配色号".into());
    }
    if !matches!(proof.data_version, 1343 | 3465 | 3955) {
        return Err("地图版本不受支持".into());
    }
    let dimension = if proof.data_version == 1343 {
        Value::Byte(match proof.dimension.as_str() {
            "minecraft:the_nether" => -1,
            "minecraft:overworld" => 0,
            "minecraft:the_end" => 1,
            _ => return Err("1.12 地图不支持该维度".into()),
        })
    } else {
        Value::String(proof.dimension.clone())
    };
    let mut fields = vec![
        Field {
            name: "dimension".into(),
            value: dimension,
        },
        Field {
            name: "xCenter".into(),
            value: Value::Int(0),
        },
        Field {
            name: "zCenter".into(),
            value: Value::Int(0),
        },
        Field {
            name: "scale".into(),
            value: Value::Byte(0),
        },
        Field {
            name: "colors".into(),
            value: Value::ByteArray(proof.colors.clone()),
        },
        Field {
            name: "trackingPosition".into(),
            value: Value::Byte(0),
        },
        Field {
            name: "unlimitedTracking".into(),
            value: Value::Byte(0),
        },
    ];
    if proof.data_version == 1343 {
        fields.push(Field {
            name: "width".into(),
            value: Value::Short(128),
        });
        fields.push(Field {
            name: "height".into(),
            value: Value::Short(128),
        });
    } else {
        fields.push(Field {
            name: "locked".into(),
            value: Value::Byte(1),
        });
        fields.push(Field {
            name: "banners".into(),
            value: Value::List(List {
                element_type: 10,
                items: Vec::new(),
            }),
        });
        fields.push(Field {
            name: "frames".into(),
            value: Value::List(List {
                element_type: 10,
                items: Vec::new(),
            }),
        });
    }
    let mut doc = Document::new(vec![10, 0, 0, 0]).map_err(|e| e.to_string())?;
    doc.apply(&Operation::InsertField {
        path: String::new(),
        name: "data".into(),
        value: Value::Compound(fields),
    })
    .map_err(|e| e.to_string())?;
    if proof.data_version != 1343 {
        doc.apply(&Operation::InsertField {
            path: String::new(),
            name: "DataVersion".into(),
            value: Value::Int(proof.data_version),
        })
        .map_err(|e| e.to_string())?;
    }
    Ok(doc.bytes().to_vec())
}
fn decode_map(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut raw = Vec::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(bytes)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)
            .map_err(|e| e.to_string())?;
    } else if bytes.len() >= 2
        && bytes[0] & 15 == 8
        && u16::from_be_bytes([bytes[0], bytes[1]]) % 31 == 0
    {
        flate2::read::ZlibDecoder::new(bytes)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)
            .map_err(|e| e.to_string())?;
    } else {
        raw.extend_from_slice(bytes);
    }
    if raw.len() > 16 * 1024 * 1024 {
        return Err("地图 NBT 解压超过预算".into());
    }
    Ok(raw)
}
pub fn create_bytes(proof: &CreateProof) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let raw = raw_map(proof)?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}
pub fn verify_created(proof: &CreateProof, bytes: &[u8]) -> Result<(), String> {
    if bytes != create_bytes(proof)? {
        return Err("地图新建内容与领域证明不一致或含尾随数据".into());
    }
    Ok(())
}
pub fn verify_deleted(proof: &DeleteProof, bytes: &[u8]) -> Result<(), String> {
    use crate::nbt_document::{Document, Value};
    if proof.id < 0 || format!("{:x}", Sha256::digest(bytes)) != proof.source_sha {
        return Err("待删除地图源摘要不匹配".into());
    }
    let doc = Document::new(decode_map(bytes)?).map_err(|e| e.to_string())?;
    let version_ok = match doc.inspect("/DataVersion") {
        Ok(Value::Int(v)) => v == proof.data_version,
        Err(_) => proof.data_version == 1343,
        _ => false,
    };
    let dimension_ok = if proof.data_version == 1343 {
        matches!(doc.inspect("/data/dimension"), Ok(Value::Byte(-1..=1)))
    } else {
        matches!(doc.inspect("/data/dimension"),Ok(Value::String(v)) if !v.is_empty())
    };
    let dims = if proof.data_version == 1343 {
        match (doc.inspect("/data/width"), doc.inspect("/data/height")) {
            (Ok(Value::Short(w)), Ok(Value::Short(h))) if w > 0 && h > 0 => {
                Some((w as usize, h as usize))
            }
            _ => None,
        }
    } else {
        Some((128, 128))
    };
    let colors_ok = match (dims, doc.inspect("/data/colors")) {
        (Some((w, h)), Ok(Value::ByteArray(v))) => w.checked_mul(h) == Some(v.len()),
        _ => false,
    };
    if !matches!(proof.data_version, 1343 | 3465 | 3955)
        || !version_ok
        || !dimension_ok
        || !colors_ok
        || !matches!(doc.inspect("/data/scale"),Ok(Value::Byte(v)) if (0..=4).contains(&v))
    {
        return Err("待删除文件不是对应版本的有效地图".into());
    }
    Ok(())
}
/// 旧版使用未压缩根 map Short，现代 SavedData 使用 gzip data.map Int。
pub fn counter_bytes(version: i32, last: i32) -> Result<Vec<u8>, String> {
    use crate::nbt_document::{Document, Field, Operation, Value};
    use std::io::Write;
    if last < 0
        || !matches!(version, 1343 | 3465 | 3955)
        || (version == 1343 && last > i16::MAX as i32)
    {
        return Err("地图计数无效或超出版本范围".into());
    }
    let mut doc = Document::new(vec![10, 0, 0, 0]).map_err(|e| e.to_string())?;
    if version == 1343 {
        doc.apply(&Operation::InsertField {
            path: String::new(),
            name: "map".into(),
            value: Value::Short(last as i16),
        })
        .map_err(|e| e.to_string())?;
        return Ok(doc.bytes().to_vec());
    }
    doc.apply(&Operation::InsertField {
        path: String::new(),
        name: "data".into(),
        value: Value::Compound(vec![Field {
            name: "map".into(),
            value: Value::Int(last),
        }]),
    })
    .map_err(|e| e.to_string())?;
    doc.apply(&Operation::InsertField {
        path: String::new(),
        name: "DataVersion".into(),
        value: Value::Int(version),
    })
    .map_err(|e| e.to_string())?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(doc.bytes()).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}
fn verify_counter_version(doc: &crate::nbt_document::Document, version: i32) -> Result<(), String> {
    use crate::nbt_document::Value;
    match doc.inspect("/DataVersion") {
        Ok(Value::Int(v)) if v == version => Ok(()),
        Err(_) if version == 1343 => Ok(()),
        _ => Err("idcounts DataVersion 与世界不一致或无法确认".into()),
    }
}
/// 根据源目录和 idcounts 准备新地图与计数器候选；不写世界。
pub fn prepare_import(
    world: &crate::WorldSummary,
    dimension: &str,
    catalog: &Catalog,
    mosaic: &Mosaic,
    source_image: (std::path::PathBuf, String),
    memory_budget: usize,
    cancelled: impl FnMut() -> bool,
) -> Result<Candidate, String> {
    prepare_import_after(
        world,
        dimension,
        catalog,
        mosaic,
        source_image,
        &[],
        memory_budget,
        cancelled,
    )
}
/// 同一世界的未保存动作占用真实 ID；新候选仍以磁盘 idcounts 为唯一原始来源。
pub fn prepare_import_after(
    world: &crate::WorldSummary,
    dimension: &str,
    catalog: &Catalog,
    mosaic: &Mosaic,
    source_image: (std::path::PathBuf, String),
    pending: &[Candidate],
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Candidate, String> {
    if cancelled() {
        return Err("地图候选准备已取消".into());
    }
    let (reserved_last, reserved_bytes) = if pending.is_empty() {
        (-1, 0)
    } else {
        let reserved = compose_cancellable(
            world,
            &pending.iter().collect::<Vec<_>>(),
            memory_budget,
            &mut cancelled,
        )?;
        let last = reserved
            .changes
            .iter()
            .filter_map(|change| match change {
                Change::Create { proof, .. } | Change::Discarded { proof } => Some(proof.id),
                Change::Delete { .. } => None,
            })
            .max()
            .unwrap_or(-1);
        (
            last,
            action_bytes(&reserved)?
                .checked_mul(4)
                .ok_or("地图动作工作集溢出")?,
        )
    };
    prepare_import_reserved(
        world,
        dimension,
        catalog,
        mosaic,
        source_image,
        reserved_last,
        reserved_bytes,
        memory_budget,
        cancelled,
    )
}
pub(crate) fn prepare_import_reserved(
    world: &crate::WorldSummary,
    dimension: &str,
    catalog: &Catalog,
    mosaic: &Mosaic,
    source_image: (std::path::PathBuf, String),
    reserved_last: i32,
    reserved_bytes: usize,
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Candidate, String> {
    use crate::nbt_document::{Operation, Value};
    if !matches!(world.data_version, Some(1343 | 3465 | 3955))
        || world.data_version != Some(mosaic.data_version)
        || !world.dimensions.iter().any(|d| d.name == dimension)
    {
        return Err("地图世界版本或维度不受支持".into());
    }
    let estimate = mosaic
        .tiles
        .len()
        .checked_mul(128 * 128 * 4)
        .ok_or("地图候选内存估算溢出")?;
    if estimate
        .checked_add(reserved_bytes)
        .ok_or("地图候选工作集溢出")?
        > memory_budget
    {
        return Err("地图候选超过内存预算".into());
    }
    if cancelled() {
        return Err("地图候选准备已取消".into());
    }
    let version = mosaic.data_version;
    let path = world.root.join("data/idcounts.dat");
    let existing = match std::fs::symlink_metadata(&path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err("idcounts 不是普通文件".into());
            }
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.to_string()),
    };
    if !existing && !catalog.entries.is_empty() {
        return Err("地图文件已存在但 idcounts 缺失；拒绝猜测重建".into());
    }
    let mut file = if existing {
        Some(
            crate::nbt_file::FileDocument::open(&world.root, &path, false)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    if let Some(file) = &file {
        verify_counter_version(&file.document, version)?;
    }
    let last = if let Some(file) = &file {
        if version == 1343 {
            match file.document.inspect("/map") {
                Ok(Value::Short(v)) => i32::from(v),
                _ => return Err("旧版 idcounts 的 /map 不是 Short".into()),
            }
        } else {
            match file.document.inspect("/data/map") {
                Ok(Value::Int(v)) => v,
                _ => return Err("现代 idcounts 的 /data/map 不是 Int".into()),
            }
        }
    } else {
        -1
    };
    let ids = allocate(catalog, last.max(reserved_last), mosaic.tiles.len())?;
    let next = *ids.last().ok_or("没有可导入的地图块")?;
    if version == 1343 && next > i16::MAX as i32 {
        return Err("旧版地图计数超出 Short 范围".into());
    }
    let counter = if let Some(file) = &mut file {
        file.document
            .apply(&Operation::Replace {
                path: if version == 1343 { "/map" } else { "/data/map" }.into(),
                value: if version == 1343 {
                    Value::Short(next as i16)
                } else {
                    Value::Int(next)
                },
            })
            .map_err(|e| e.to_string())?;
        Counter::Existing(file.candidate().map_err(|e| e.to_string())?)
    } else {
        Counter::New {
            last: next,
            bytes: counter_bytes(version, next)?,
        }
    };
    let mut changes = Vec::with_capacity(ids.len());
    for (id, tile) in ids.into_iter().zip(&mosaic.tiles) {
        if cancelled() {
            return Err("地图候选准备已取消".into());
        }
        let proof = CreateProof {
            id,
            data_version: version,
            dimension: dimension.into(),
            colors: tile.colors.clone(),
        };
        let bytes = create_bytes(&proof)?;
        changes.push(Change::Create { proof, bytes });
    }
    Ok(Candidate {
        root: world.root.clone(),
        data_version: version,
        counter: Some(counter),
        changes,
        source_images: vec![source_image],
    })
}
pub fn prepare_delete(
    world: &crate::WorldSummary,
    entry: &crate::map_catalog::Entry,
) -> Result<Candidate, String> {
    let version = world
        .data_version
        .filter(|v| matches!(*v, 1343 | 3465 | 3955))
        .ok_or("地图版本不受支持")?;
    let expected = world
        .root
        .join("data")
        .join(format!("map_{}.dat", entry.id));
    if expected != entry.path {
        return Err("地图来源路径不属于当前世界".into());
    }
    let meta = std::fs::symlink_metadata(&expected).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("待删除地图不是普通文件".into());
    }
    let bytes = std::fs::read(&expected).map_err(|e| e.to_string())?;
    let proof = DeleteProof {
        id: entry.id,
        data_version: version,
        source_sha: entry.sha256.clone(),
    };
    verify_deleted(&proof, &bytes)?;
    Ok(Candidate {
        root: world.root.clone(),
        data_version: version,
        counter: None,
        changes: vec![Change::Delete { proof }],
        source_images: Vec::new(),
    })
}
/// 删除当前修订中的地图，包括尚未保存的新图；合并时保留分配证明并消除文件操作。
pub fn prepare_delete_after(
    world: &crate::WorldSummary,
    entry: &crate::map_catalog::Entry,
    pending: &[Candidate],
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Candidate, String> {
    let view = View::from_candidates(&world.root, pending);
    let candidate = match view.entries.get(&entry.id) {
        Some(ViewEntry::Created { version, bytes, .. }) => {
            if world.data_version != Some(*version)
                || entry.path
                    != world
                        .root
                        .join("data")
                        .join(format!("map_{}.dat", entry.id))
            {
                return Err("地图删除来源世界或版本不一致".into());
            }
            let proof = DeleteProof {
                id: entry.id,
                data_version: *version,
                source_sha: entry.sha256.clone(),
            };
            verify_deleted(&proof, bytes)?;
            Candidate {
                root: world.root.clone(),
                data_version: *version,
                counter: None,
                changes: vec![Change::Delete { proof }],
                source_images: Vec::new(),
            }
        }
        Some(ViewEntry::Deleted { .. }) => return Err("地图已在当前修订删除".into()),
        None => prepare_delete(world, entry)?,
    };
    let mut group = pending.iter().collect::<Vec<_>>();
    group.push(&candidate);
    compose_cancellable(world, &group, memory_budget, &mut cancelled)?;
    Ok(candidate)
}

impl Candidate {
    /// 事务准备时重新验证所有源身份和目标不存在；恢复时另用单项纯证明验证。
    pub fn verify_world(&self, world: &crate::WorldSummary) -> Result<(), String> {
        self.verify_world_cancellable(world, || false)
    }
    pub fn verify_world_cancellable(
        &self,
        world: &crate::WorldSummary,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), String> {
        if cancelled() {
            return Err("地图来源复核已取消".into());
        }
        if self.root != world.root
            || !matches!(self.data_version, 1343 | 3465 | 3955)
            || world.data_version != Some(self.data_version)
        {
            return Err("地图候选世界或版本已改变".into());
        }
        for (path, expected) in &self.source_images {
            if cancelled() {
                return Err("地图图片源复核已取消".into());
            }
            let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 64 * 1024 * 1024 {
                return Err("地图导入图片源类型或大小已变".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if format!("{:x}", Sha256::digest(&bytes)) != *expected {
                return Err("地图导入图片源摘要已变".into());
            }
        }
        let dir = world.root.join("data");
        let directory_exists = match std::fs::symlink_metadata(&dir) {
            Ok(meta) => {
                if meta.file_type().is_symlink() || !meta.is_dir() {
                    return Err("地图目录不是普通目录".into());
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.to_string()),
        };
        if !directory_exists
            && (!matches!(&self.counter, Some(Counter::New { .. }))
                || self
                    .changes
                    .iter()
                    .any(|v| matches!(v, Change::Delete { .. })))
        {
            return Err("缺失地图目录仅允许首次创建".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut created = 0usize;
        for change in &self.changes {
            if cancelled() {
                return Err("地图目标复核已取消".into());
            }
            match change {
                Change::Create { proof, bytes } => {
                    if !ids.insert(proof.id) {
                        return Err("地图候选 ID 重复".into());
                    }
                    if !world.dimensions.iter().any(|v| v.name == proof.dimension) {
                        return Err("地图维度无效".into());
                    }
                    verify_created(proof, bytes)?;
                    let path = dir.join(format!("map_{}.dat", proof.id));
                    match std::fs::symlink_metadata(path) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Ok(_) => return Err("地图新建目标已存在".into()),
                        Err(e) => return Err(e.to_string()),
                    };
                    created += 1;
                }
                Change::Delete { proof } => {
                    if !ids.insert(proof.id) {
                        return Err("地图候选 ID 重复".into());
                    }
                    let path = dir.join(format!("map_{}.dat", proof.id));
                    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                    if meta.file_type().is_symlink() || !meta.is_file() {
                        return Err("地图删除源不是普通文件".into());
                    }
                    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                    verify_deleted(proof, &bytes)?;
                }
                Change::Discarded { proof } => {
                    if !ids.insert(proof.id) {
                        return Err("地图候选 ID 重复".into());
                    }
                    raw_map(proof)?;
                    let path = dir.join(format!("map_{}.dat", proof.id));
                    match std::fs::symlink_metadata(path) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Ok(_) => return Err("已放弃的新地图目标已存在".into()),
                        Err(e) => return Err(e.to_string()),
                    }
                    created += 1;
                }
            }
        }
        let proofs = self
            .changes
            .iter()
            .map(|change| match change {
                Change::Create { proof, .. } => Proof::Create(proof.clone()),
                Change::Delete { proof } => Proof::Delete(proof.clone()),
                Change::Discarded { proof } => Proof::Discarded(proof.clone()),
            })
            .collect::<Vec<_>>();
        let source = crate::entity_editing::DiskIdentitySource;
        let (old, next) = match &self.counter {
            Some(Counter::Existing(counter)) => {
                if counter.root != world.root || counter.path != dir.join("idcounts.dat") {
                    return Err("地图计数候选路径无效".into());
                }
                let bytes = std::fs::read(&counter.path).map_err(|e| e.to_string())?;
                if format!("{:x}", Sha256::digest(&bytes)) != counter.source_sha {
                    return Err("idcounts 源已变化".into());
                }
                (Some(bytes), Some(counter.bytes.clone()))
            }
            Some(Counter::New { last, bytes }) => {
                if bytes != &counter_bytes(self.data_version, *last)? {
                    return Err("新地图计数内容不匹配".into());
                }
                (None, Some(bytes.clone()))
            }
            None => (None, None),
        };
        if created > 0 && next.is_none() {
            return Err("新建地图缺少 idcounts 候选".into());
        }
        verify_group_with(&source, world, &proofs, old.as_deref(), next.as_deref())?;
        Ok(())
    }
}
/// 恢复时重建事务前的地图目录与 idcounts，验证整组分配关系。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Proof {
    Create(CreateProof),
    Delete(DeleteProof),
    Discarded(CreateProof),
}
pub fn verify_group_with(
    source: &impl crate::entity_editing::IdentitySource,
    world: &crate::WorldSummary,
    proofs: &[Proof],
    original_counter: Option<&[u8]>,
    candidate_counter: Option<&[u8]>,
) -> Result<(), String> {
    use crate::nbt_document::{Document, Value};
    if !matches!(world.data_version, Some(1343 | 3465 | 3955)) || proofs.is_empty() {
        return Err("地图事务版本或证明无效".into());
    }
    let version = world.data_version.ok_or("地图版本缺失")?;
    let dir = world.root.join("data");
    let directory_exists = source.is_directory(&dir).map_err(|e| e.to_string())?;
    let names = if directory_exists {
        source
            .directory_members(&dir)
            .map_err(|e| e.to_string())?
            .ok_or("地图原始目录不可读")?
    } else {
        Vec::new()
    };
    if !directory_exists
        && (original_counter.is_some() || proofs.iter().any(|p| matches!(p, Proof::Delete(_))))
    {
        return Err("缺失地图目录仅允许首次创建".into());
    }
    let mut max = -1i32;
    for name in names {
        if let Some(n) = name
            .strip_prefix("map_")
            .and_then(|v| v.strip_suffix(".dat"))
        {
            if let Ok(id) = n.parse::<i32>() {
                if id >= 0 && id.to_string() == n {
                    max = max.max(id);
                }
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut creates = Vec::new();
    for proof in proofs {
        match proof {
            Proof::Create(p) | Proof::Discarded(p) => {
                if p.data_version != version
                    || p.id < 0
                    || !seen.insert(p.id)
                    || !world.dimensions.iter().any(|d| d.name == p.dimension)
                {
                    return Err("地图创建 ID 或维度无效".into());
                }
                if directory_exists
                    && source
                        .read(&dir.join(format!("map_{}.dat", p.id)))
                        .map_err(|e| e.to_string())?
                        .is_some()
                {
                    return Err("地图创建目标原本已存在".into());
                }
                raw_map(p)?;
                creates.push(p.id);
            }
            Proof::Delete(p) => {
                if p.data_version != version || p.id < 0 || !seen.insert(p.id) {
                    return Err("地图删除 ID 重复或无效".into());
                }
                let bytes = source
                    .read(&dir.join(format!("map_{}.dat", p.id)))
                    .map_err(|e| e.to_string())?
                    .ok_or("地图删除源不存在")?;
                verify_deleted(p, &bytes)?;
            }
        }
    }
    if creates.is_empty() {
        if original_counter.is_some() || candidate_counter.is_some() {
            return Err("纯删除不应修改地图 ID 计数".into());
        }
        return Ok(());
    }
    let next = candidate_counter.ok_or("地图 ID 候选缺失")?;
    let actual = if directory_exists {
        source
            .read(&dir.join("idcounts.dat"))
            .map_err(|e| e.to_string())?
    } else {
        None
    };
    if actual.as_deref() != original_counter {
        return Err("地图 ID 原计数与原始视图不一致".into());
    }
    let previous = if let Some(old) = original_counter {
        let doc = Document::new(decode_map(old)?).map_err(|e| e.to_string())?;
        verify_counter_version(&doc, version)?;
        if version == 1343 {
            match doc.inspect("/map") {
                Ok(Value::Short(v)) if v >= -1 => i32::from(v),
                _ => return Err("旧版地图 ID 原计数格式不受支持".into()),
            }
        } else {
            match doc.inspect("/data/map") {
                Ok(Value::Int(v)) if v >= -1 => v,
                _ => return Err("地图 ID 原计数格式不受支持".into()),
            }
        }
    } else {
        if max >= 0 {
            return Err("已有地图但 idcounts 缺失，拒绝猜测重建".into());
        }
        -1
    };
    let mut next_id = max.max(previous);
    for id in creates {
        next_id = next_id.checked_add(1).ok_or("地图 ID 已耗尽")?;
        if id != next_id {
            return Err("地图 ID 分配不连续或与原始目录冲突".into());
        }
    }
    if version == 1343 && next_id > i16::MAX as i32 {
        return Err("旧版地图 ID 超出 Short 范围".into());
    }
    verify_counter_transition(version, original_counter, next, next_id)
}
fn verify_counter_transition(
    version: i32,
    original_counter: Option<&[u8]>,
    next: &[u8],
    next_id: i32,
) -> Result<(), String> {
    use crate::nbt_document::{Document, Operation, Value};
    if next_id < 0 || (version == 1343 && next_id > i16::MAX as i32) {
        return Err("地图 ID 计数超出版本范围".into());
    }
    if let Some(old) = original_counter {
        crate::nbt_file::verify_transition(old, next).map_err(|e| e.to_string())?;
        let mut doc = Document::new(decode_map(old)?).map_err(|e| e.to_string())?;
        doc.apply(&Operation::Replace {
            path: if version == 1343 { "/map" } else { "/data/map" }.into(),
            value: if version == 1343 {
                Value::Short(next_id as i16)
            } else {
                Value::Int(next_id)
            },
        })
        .map_err(|e| e.to_string())?;
        if decode_map(next)? != doc.bytes() {
            return Err("地图 ID 候选含未经批准的其他更改".into());
        }
    } else if next != counter_bytes(version, next_id)? {
        return Err("新地图 ID 计数不符合对应版本原版结构".into());
    }
    Ok(())
}

fn action_bytes(candidate: &Candidate) -> Result<usize, String> {
    candidate.changes.iter().try_fold(0usize, |total, change| {
        let bytes = match change {
            Change::Create { proof, bytes } => bytes.len().checked_add(proof.colors.len()),
            Change::Delete { proof } => Some(proof.source_sha.len()),
            Change::Discarded { proof } => Some(proof.colors.len()),
        }
        .and_then(|n| n.checked_add(std::mem::size_of::<Change>()))
        .ok_or("地图动作工作集溢出")?;
        total
            .checked_add(bytes)
            .ok_or_else(|| "地图动作工作集溢出".into())
    })
}

/// 将同一世界的动作按顺序联合预检，仅保留最终计数写入，图片来源全部保留。
/// 每个中间计数也核对原始内容和精确的 /map 更新，禁止用最终计数掩盖坏候选。
pub fn compose(
    world: &crate::WorldSummary,
    candidates: &[&Candidate],
    memory_budget: usize,
) -> Result<Candidate, String> {
    compose_cancellable(world, candidates, memory_budget, || false)
}
pub fn compose_cancellable(
    world: &crate::WorldSummary,
    candidates: &[&Candidate],
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Candidate, String> {
    if cancelled() {
        return Err("地图动作合并已取消".into());
    }
    if candidates.is_empty() {
        return Err("地图动作组为空".into());
    }
    let version = world.data_version.ok_or("世界缺少数据版本")?;
    let mut combined = Candidate {
        root: world.root.clone(),
        data_version: version,
        counter: None,
        changes: Vec::new(),
        source_images: Vec::new(),
    };
    let mut retained = 0usize;
    let mut original_counter: Option<Option<Vec<u8>>> = None;
    let mut targets = std::collections::BTreeMap::<i32, usize>::new();
    for candidate in candidates {
        if cancelled() {
            return Err("地图动作合并已取消".into());
        }
        if candidate.root != world.root || candidate.data_version != version {
            return Err("地图动作属于不同世界或数据版本".into());
        }
        if candidate.changes.is_empty() {
            return Err("地图动作为空".into());
        }
        let cost = action_bytes(candidate)?;
        retained = retained.checked_add(cost).ok_or("地图动作工作集溢出")?;
        if retained.checked_mul(4).ok_or("地图动作工作集溢出")? > memory_budget {
            return Err("地图动作组超过任务预算".into());
        }
        let created = candidate
            .changes
            .iter()
            .filter_map(|change| match change {
                Change::Create { proof, .. } | Change::Discarded { proof } => Some(proof.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        if created.is_empty() != candidate.counter.is_none() {
            return Err("地图创建动作与计数候选不一致".into());
        }
        if let Some(counter) = &candidate.counter {
            let (old, next) = match counter {
                Counter::Existing(c) => {
                    let path = world.root.join("data/idcounts.dat");
                    if c.root != world.root || c.path != path {
                        return Err("地图计数候选来源路径无效".into());
                    }
                    let raw = std::fs::read(path).map_err(|error| error.to_string())?;
                    if format!("{:x}", Sha256::digest(&raw)) != c.source_sha {
                        return Err("地图计数原始来源已变化".into());
                    }
                    (Some(raw), c.bytes.as_slice())
                }
                Counter::New { last, bytes } => {
                    if Some(*last) != created.last().copied() {
                        return Err("新地图计数与动作 ID 不一致".into());
                    }
                    (None, bytes.as_slice())
                }
            };
            if original_counter
                .as_ref()
                .is_some_and(|expected| expected != &old)
            {
                return Err("地图动作的原始计数来源不一致".into());
            }
            verify_counter_transition(
                version,
                old.as_deref(),
                next,
                *created.last().ok_or("地图创建为空")?,
            )?;
            original_counter = Some(old);
            combined.counter = Some(counter.clone());
        }
        for change in &candidate.changes {
            if cancelled() {
                return Err("地图动作合并已取消".into());
            }
            let id = match change {
                Change::Create { proof, .. } | Change::Discarded { proof } => proof.id,
                Change::Delete { proof } => proof.id,
            };
            if let Some(&index) = targets.get(&id) {
                let (
                    Change::Create {
                        proof: created,
                        bytes,
                    },
                    Change::Delete { proof: deleted },
                ) = (&combined.changes[index], change)
                else {
                    return Err("地图 ID 与已有动作重复".into());
                };
                if created.data_version != deleted.data_version {
                    return Err("地图删除与新建版本不同".into());
                }
                verify_deleted(deleted, bytes)?;
                combined.changes[index] = Change::Discarded {
                    proof: created.clone(),
                };
            } else {
                targets.insert(id, combined.changes.len());
                combined.changes.push(change.clone());
            }
        }
        combined
            .source_images
            .extend(candidate.source_images.iter().cloned());
    }
    combined.verify_world_cancellable(world, &mut cancelled)?;
    Ok(combined)
}

/// 按目标拼图尺寸缩放；等比例模式将剩余区域保持透明。
pub fn resize_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
    target_width: u32,
    target_height: u32,
    stretch: bool,
    memory_budget: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Vec<u8>, String> {
    let original = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("图片尺寸溢出")?;
    let target = (target_width as usize)
        .checked_mul(target_height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("目标图片尺寸溢出")?;
    if width == 0
        || height == 0
        || target_width == 0
        || target_height == 0
        || original != rgba.len()
        || original
            .checked_add(target)
            .is_none_or(|n| n > memory_budget)
    {
        return Err("缩放图片尺寸或内存预算无效".into());
    }
    let (draw_w, draw_h) = if stretch {
        (target_width, target_height)
    } else {
        let width_limited =
            (target_width as u64) * (height as u64) <= (target_height as u64) * (width as u64);
        if width_limited {
            (
                target_width,
                ((height as u64) * (target_width as u64) / (width as u64)).max(1) as u32,
            )
        } else {
            (
                ((width as u64) * (target_height as u64) / (height as u64)).max(1) as u32,
                target_height,
            )
        }
    };
    let left = (target_width - draw_w) / 2;
    let top = (target_height - draw_h) / 2;
    let mut output = vec![0u8; target];
    for y in 0..draw_h {
        if cancelled() {
            return Err("图片缩放已取消".into());
        }
        let sy = (y as u64 * height as u64 / draw_h as u64) as usize;
        for x in 0..draw_w {
            let sx = (x as u64 * width as u64 / draw_w as u64) as usize;
            let a = (sy * width as usize + sx) * 4;
            let b = ((top + y) as usize * target_width as usize + (left + x) as usize) * 4;
            output[b..b + 4].copy_from_slice(&rgba[a..a + 4]);
        }
    }
    Ok(output)
}
