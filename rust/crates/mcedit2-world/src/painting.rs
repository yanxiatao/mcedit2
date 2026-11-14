//! 挂画的实际注册定义与悬挂位置。注册表来自源版本及世界启用的数据包，图像来自客户端资源。
use crate::{
    entity_catalog::EntityVisual,
    nbt_document::{Field, Value},
    world_data::RegistryLayer,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Variant {
    pub width: u8,
    pub height: u8,
    pub asset_id: String,
    pub motive: Option<String>,
    pub legacy_uv: Option<[u16; 2]>,
}
impl Variant {
    pub(crate) fn validate(self) -> Result<Self, String> {
        if !(1..=16).contains(&self.width) || !(1..=16).contains(&self.height) {
            return Err("挂画 width / height 必须为 1 至 16 格".into());
        }
        crate::world_data::registry_id(&self.asset_id).map_err(|e| e.to_string())?;
        Ok(self)
    }
    fn inline(fields: &[Field]) -> Result<Self, String> {
        let get = |key| {
            fields
                .iter()
                .find(|f| f.name == key)
                .map(|f| &f.value)
                .ok_or_else(|| format!("内联挂画缺少 {key}"))
        };
        let size = |key| match get(key)? {
            Value::Int(n) => u8::try_from(*n).map_err(|_| format!("挂画 {key} 超出范围")),
            _ => Err(format!("挂画 {key} 必须为 Int")),
        };
        let Value::String(asset) = get("asset_id")? else {
            return Err("挂画 asset_id 必须为 String".into());
        };
        Self {
            width: size("width")?,
            height: size("height")?,
            asset_id: qualify(asset)?,
            motive: None,
            legacy_uv: None,
        }
        .validate()
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
#[derive(Debug, Clone)]
pub struct Catalog(pub BTreeMap<String, Variant>);
impl Catalog {
    pub fn builtin(version: i32) -> Result<Arc<Self>, String> {
        static OLD: OnceLock<Arc<Catalog>> = OnceLock::new();
        static MID: OnceLock<Arc<Catalog>> = OnceLock::new();
        static NEW: OnceLock<Arc<Catalog>> = OnceLock::new();
        let (slot, bytes): (&OnceLock<Arc<Self>>, &[u8]) = match version {
            1343 => (&OLD, include_bytes!("../assets/paintings-1.12.2.json")),
            3465 => (&MID, include_bytes!("../assets/paintings-1.20.1.json")),
            3955 => (&NEW, include_bytes!("../assets/paintings-1.21.1.json")),
            _ => return Err(format!("尚未核对 DataVersion {version} 的挂画注册表")),
        };
        Ok(Arc::clone(slot.get_or_init(|| {
            Arc::new(Self(
                serde_json::from_slice(bytes).expect("内建挂画目录必须有效"),
            ))
        })))
    }
    pub fn with_layers(layers: &[RegistryLayer]) -> Result<Arc<Self>, String> {
        if layers.is_empty() {
            return Self::builtin(3955);
        }
        let values = crate::world_data::apply_registry_layers(
            "painting_variant",
            Self::builtin(3955)?.0.clone(),
            layers,
            |id, bytes| {
                let value: Variant =
                    serde_json::from_slice(bytes).map_err(|e| format!("挂画定义 {id}：{e}"))?;
                let mut value = value;
                value.asset_id = qualify(&value.asset_id)?;
                value.validate()
            },
        )?;
        Ok(Arc::new(Self(values)))
    }
}
pub fn is_painting(id: &str) -> bool {
    matches!(id, "Painting" | "minecraft:painting")
}
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub variant: Variant,
    /// South / West / North / East；不读取实体 Rotation 推测悬挂方向。
    pub facing: u8,
    pub tile: [i32; 3],
}
impl Source {
    pub fn read(visual: &EntityVisual, catalog: &Catalog) -> Result<Self, String> {
        Self::read_bound(visual, catalog, &BTreeMap::new())
    }
    /// 结构捕获的定义覆盖内建目录；只借用目录，不为每个实体复制整份注册表。
    pub fn read_bound(
        visual: &EntityVisual,
        catalog: &Catalog,
        definitions: &BTreeMap<String, Variant>,
    ) -> Result<Self, String> {
        let version = visual.source_version.ok_or("挂画缺少源 DataVersion")?;
        let variant = match version {
            1343 => {
                let Value::String(motive) = visual.source_field("Motive")? else {
                    return Err("Motive 必须为 String".into());
                };
                definitions
                    .values()
                    .chain(catalog.0.values())
                    .find(|v| v.motive.as_deref() == Some(&motive))
                    .cloned()
                    .ok_or_else(|| format!("未注册的挂画 Motive：{motive}"))?
            }
            3465 | 3955 => match visual.source_field("variant")? {
                Value::String(id) => {
                    let id = qualify(&id)?;
                    definitions
                        .get(&id)
                        .or_else(|| catalog.0.get(&id))
                        .cloned()
                        .ok_or_else(|| format!("未注册的挂画 variant：{id}"))?
                }
                Value::Compound(fields) if version == 3955 => Variant::inline(&fields)?,
                _ => return Err("variant 必须为注册 ID；1.21.1 也接受内联定义".into()),
            },
            _ => return Err(format!("尚未核对 DataVersion {version} 的挂画字段")),
        };
        let key = if version == 1343 { "Facing" } else { "facing" };
        let facing = match visual.source_field_optional(key)? {
            None => 0, // 三版原生 getByte 在字段不存在时均为 South。
            Some(Value::Byte(n)) if (0..4).contains(&n) => n as u8,
            _ => return Err(format!("{key} 必须为 0 至 3 的 Byte")),
        };
        let mut tile = [0; 3];
        for (i, key) in ["TileX", "TileY", "TileZ"].iter().enumerate() {
            let Value::Int(n) = visual.source_field(key)? else {
                return Err(format!("{key} 必须为 Int"));
            };
            tile[i] = n;
        }
        Ok(Self {
            variant,
            facing,
            tile,
        })
    }
    pub fn center(&self) -> [f64; 3] {
        let [dx, dz] = [[0., 1.], [-1., 0.], [0., -1.], [1., 0.]][self.facing as usize];
        let horizontal = if self.variant.width % 2 == 0 { 0.5 } else { 0. };
        let vertical = if self.variant.height % 2 == 0 {
            0.5
        } else {
            0.
        };
        [
            f64::from(self.tile[0]) + 0.5 - dx * 0.46875 + dz * horizontal,
            f64::from(self.tile[1]) + 0.5 + vertical,
            f64::from(self.tile[2]) + 0.5 - dz * 0.46875 - dx * horizontal,
        ]
    }
    /// 每个一格画片各自取光照；使用实际定义尺寸和悬挂基底，不依赖画作 ID。
    pub fn light_position(&self, x: u8, y: u8) -> Result<[i32; 3], String> {
        if x >= self.variant.width || y >= self.variant.height {
            return Err("挂画光照画片超出实际定义尺寸".into());
        }
        let local = self.rotate([
            -f32::from(self.variant.width) / 2. + f32::from(x) + 0.5,
            -f32::from(self.variant.height) / 2. + f32::from(y) + 0.5,
            0.,
        ]);
        let center = self.center();
        let mut point = [0; 3];
        for i in 0..3 {
            let value = (center[i] + f64::from(local[i])).floor();
            if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
                return Err("挂画光照源格超出可表示坐标".into());
            }
            point[i] = value as i32;
        }
        Ok(point)
    }
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let width = f64::from(self.variant.width) / 2.;
        let depth = 1. / 32.;
        let half = if self.facing % 2 == 0 {
            [width, f64::from(self.variant.height) / 2., depth]
        } else {
            [depth, f64::from(self.variant.height) / 2., width]
        };
        let center = self.center();
        (
            std::array::from_fn(|i| center[i] - half[i]),
            std::array::from_fn(|i| center[i] + half[i]),
        )
    }
    pub fn rotate(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
        match self.facing {
            0 => [-x, y, -z],
            1 => [z, y, -x],
            2 => [x, y, z],
            3 => [-z, y, x],
            _ => unreachable!("源方向已核实"),
        }
    }
}
