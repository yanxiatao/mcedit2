//! 地图标记的原始来源、注册定义和原版量化坐标；不生成在线玩家或臆造世界标记。
use crate::nbt_document::{Field, Value};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Definition {
    #[serde(skip)]
    pub id: String,
    pub asset_id: Option<String>,
    pub legacy_icon: Option<u8>,
    pub show_on_frame: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Decoration {
    pub key: String,
    pub definition: Definition,
    pub x: i8,
    pub y: i8,
    pub rotation: u8,
    pub name: Option<serde_json::Value>,
}
pub fn catalog(version: i32) -> Result<&'static BTreeMap<String, Definition>, String> {
    static OLD: OnceLock<BTreeMap<String, Definition>> = OnceLock::new();
    static MID: OnceLock<BTreeMap<String, Definition>> = OnceLock::new();
    static NEW: OnceLock<BTreeMap<String, Definition>> = OnceLock::new();
    let (cache, json) = match version {
        1343 => (
            &OLD,
            include_str!("../assets/map-decoration-types-1.12.2.json"),
        ),
        3465 => (
            &MID,
            include_str!("../assets/map-decoration-types-1.20.1.json"),
        ),
        3955 => (
            &NEW,
            include_str!("../assets/map-decoration-types-1.21.1.json"),
        ),
        _ => return Err(format!("尚未核对 DataVersion {version} 的地图标记")),
    };
    Ok(cache.get_or_init(|| {
        let mut entries: BTreeMap<String, Definition> =
            serde_json::from_str(json).expect("内置客户端地图标记参照无效");
        for (id, definition) in &mut entries {
            definition.id = id.clone();
        }
        entries
    }))
}
fn object(value: &Value) -> Result<&[Field], String> {
    match value {
        Value::Compound(fields) => Ok(fields),
        _ => Err("地图标记必须为 Compound".into()),
    }
}
fn field<'a>(fields: &'a [Field], name: &str) -> Option<&'a Value> {
    fields.iter().find(|f| f.name == name).map(|f| &f.value)
}
fn required<'a>(fields: &'a [Field], name: &str) -> Result<&'a Value, String> {
    field(fields, name).ok_or_else(|| format!("地图标记缺少 {name}"))
}
fn int(value: &Value) -> Result<i32, String> {
    match value {
        Value::Int(n) => Ok(*n),
        _ => Err("地图标记坐标及身份必须为 Int".into()),
    }
}
fn number(value: &Value) -> Result<f64, String> {
    let n = match value {
        Value::Double(n) => *n,
        Value::Float(n) => f64::from(*n),
        _ => return Err("地图标记坐标及角度必须为浮点数".into()),
    };
    if !n.is_finite() {
        return Err("地图标记坐标及角度不是有限数值".into());
    }
    Ok(n)
}
fn string(value: &Value) -> Result<&str, String> {
    match value {
        Value::String(s) => Ok(s),
        _ => Err("地图标记名称及资源必须为 String".into()),
    }
}
fn list<'a>(fields: &'a [Field], key: &str) -> Result<&'a [Value], String> {
    match field(fields, key) {
        None => Ok(&[]),
        Some(Value::List(list)) if list.items.iter().all(|v| matches!(v, Value::Compound(_))) => {
            Ok(&list.items)
        }
        _ => Err(format!("地图 {key} 必须为 Compound List")),
    }
}
fn qualify(id: &str) -> Result<String, String> {
    let id = if id.contains(':') {
        id.to_string()
    } else {
        format!("minecraft:{id}")
    };
    crate::world_data::registry_id(&id).map_err(|e| e.to_string())?;
    Ok(id)
}
fn named(version: i32, id: &str) -> Result<Definition, String> {
    let id = qualify(id)?;
    catalog(version)?
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("地图标记注册项不存在：{id}"))
}
fn holder(version: i32, value: &Value) -> Result<Definition, String> {
    // MapDecorationType.CODEC 为 holderByNameCodec，并非可内联 Holder。
    named(version, string(value)?)
}
fn legacy(version: i32, value: &Value) -> Result<Definition, String> {
    let Value::Byte(index) = value else {
        return Err("旧地图标记 type 必须为 Byte".into());
    };
    catalog(version)?
        .values()
        .find(|d| d.legacy_icon == Some(*index as u8))
        .cloned()
        .ok_or_else(|| format!("旧地图标记 type {index} 未注册"))
}
/// 按客户端 addDecoration 的 float 中间值、向零截断和十六档方向编码。
/// 非玩家标记越界时移除同身份旧标记，不能钳制到地图边缘。
struct Projection {
    version: i32,
    center: [i32; 2],
    scale: i8,
    unlimited_tracking: bool,
}
impl Projection {
    fn place(
        &self,
        markers: &mut Vec<Decoration>,
        mut definition: Definition,
        key: String,
        position: [f64; 2],
        angle: f64,
        name: Option<serde_json::Value>,
    ) -> Result<(), String> {
        let distance = 1i32 << self.scale;
        let [x, y] = std::array::from_fn(|i| {
            (position[i] - f64::from(self.center[i])) as f32 / distance as f32
        });
        let mut px = (f64::from(x * 2.) + 0.5) as i32 as i8;
        let mut py = (f64::from(y * 2.) + 0.5) as i32 as i8;
        let mut rotation =
            (((angle + if angle < 0. { -8. } else { 8. }) * 16. / 360.) as i32 & 15) as u8;
        if !(-63. ..=63.).contains(&x) || !(-63. ..=63.).contains(&y) {
            if definition.id != "minecraft:player"
                || (x.abs() >= 320. || y.abs() >= 320.) && !self.unlimited_tracking
            {
                markers.retain(|m| m.key != key);
                return Ok(());
            }
            definition = named(
                self.version,
                if x.abs() < 320. && y.abs() < 320. {
                    "minecraft:player_off_map"
                } else {
                    "minecraft:player_off_limits"
                },
            )?;
            rotation = 0;
            if x <= -63. {
                px = -128;
            }
            if y <= -63. {
                py = -128;
            }
            if x >= 63. {
                px = 127;
            }
            if y >= 63. {
                py = 127;
            }
        }
        let marker = Decoration {
            key,
            definition,
            x: px,
            y: py,
            rotation,
            name,
        };
        if let Some(old) = markers.iter_mut().find(|m| m.key == marker.key) {
            *old = marker;
        } else {
            markers.push(marker);
        }
        Ok(())
    }
}
fn pos(fields: &[Field], version: i32) -> Result<[i32; 3], String> {
    if version == 3955 {
        match required(fields, "pos")? {
            Value::IntArray(p) if p.len() == 3 => Ok([p[0], p[1], p[2]]),
            _ => Err("地图标记 pos 必须为三个 Int".into()),
        }
    } else {
        let p = object(required(fields, "Pos")?)?;
        Ok([
            int(required(p, "X")?)?,
            int(required(p, "Y")?)?,
            int(required(p, "Z")?)?,
        ])
    }
}
/// 现代文件中实际保存的旗帜 / 展示框；旧版文件不持久化这些运行时标记。
pub fn saved(
    data: &Value,
    version: i32,
    center: [i32; 2],
    scale: i8,
) -> Result<Vec<Decoration>, String> {
    let mut out = Vec::new();
    if version == 1343 {
        return Ok(out);
    }
    let fields = object(data)?;
    let projection = Projection {
        version,
        center,
        scale,
        unlimited_tracking: false,
    };
    for banner in list(fields, "banners")? {
        let b = object(banner)?;
        let p = pos(b, version)?;
        let color = field(b, if version == 3955 { "color" } else { "Color" })
            .map(string)
            .transpose()?
            .unwrap_or("white");
        let name = field(b, if version == 3955 { "name" } else { "Name" })
            .map(|v| {
                serde_json::from_str(string(v)?).map_err(|e| format!("地图旗帜名称组件无效：{e}"))
            })
            .transpose()?;
        projection.place(
            &mut out,
            named(version, &format!("minecraft:banner_{color}"))?,
            format!("banner-{},{},{}", p[0], p[1], p[2]),
            [f64::from(p[0]), f64::from(p[2])],
            180.,
            name,
        )?;
    }
    for frame in list(fields, "frames")? {
        let f = object(frame)?;
        let p = pos(f, version)?;
        let id = int(required(
            f,
            if version == 3955 {
                "entity_id"
            } else {
                "EntityId"
            },
        )?)?;
        let rotation = int(required(
            f,
            if version == 3955 {
                "rotation"
            } else {
                "Rotation"
            },
        )?)?;
        projection.place(
            &mut out,
            named(version, "minecraft:frame")?,
            format!("frame-{id}"),
            [f64::from(p[0]), f64::from(p[2])],
            f64::from(rotation),
            None,
        )?;
    }
    Ok(out)
}
/// 物品装饰晚于持久化标记读取；同身份已存在时原版不覆盖。
pub fn from_item(
    markers: &mut Vec<Decoration>,
    item: &Value,
    version: i32,
    center: [i32; 2],
    scale: i8,
    unlimited_tracking: bool,
) -> Result<(), String> {
    let projection = Projection {
        version,
        center,
        scale,
        unlimited_tracking,
    };
    let fields = object(item)?;
    let Some(data) = field(fields, if version == 3955 { "components" } else { "tag" }) else {
        return Ok(());
    };
    let data = object(data)?;
    let entries = if version == 3955 {
        let Some(value) = field(data, "minecraft:map_decorations") else {
            return Ok(());
        };
        object(value)?
            .iter()
            .map(|f| Ok((f.name.clone(), object(&f.value)?)))
            .collect::<Result<Vec<_>, String>>()?
    } else {
        list(data, "Decorations")?
            .iter()
            .map(|v| {
                let f = object(v)?;
                Ok((string(required(f, "id")?)?.to_string(), f))
            })
            .collect::<Result<Vec<_>, String>>()?
    };
    for (key, entry) in entries {
        if markers.iter().any(|m| m.key == key) {
            continue;
        }
        let definition = if version == 3955 {
            holder(version, required(entry, "type")?)?
        } else {
            legacy(version, required(entry, "type")?)?
        };
        projection.place(
            markers,
            definition,
            key,
            [
                number(required(entry, "x")?)?,
                number(required(entry, "z")?)?,
            ],
            number(required(
                entry,
                if version == 3955 { "rotation" } else { "rot" },
            )?)?,
            None,
        )?;
    }
    Ok(())
}
