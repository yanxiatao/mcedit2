//! 明确来源的数字 biome 气候 / 颜色表；不产生命名 ID，不写回世界注册表。
use super::{biome_colors::Kind, Tile};
use mcedit2_world::biome_source::NumericSample;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

pub(super) const ASSET: &str = "assets/mcedit2/biomes/numeric.json";
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    pub id: i32,
    pub name: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.id)
    }
}

#[cfg(test)]
pub(super) fn validate_numeric_pipeline() {
    use super::{
        biome_colors::{Context, Resources},
        TextureBank,
    };
    use mcedit2_world::{biome_source::Region, editing::parse_state, numeric_biome::Volume};
    let original = Profile::original();
    assert_eq!(original.choices().count(), 61);
    assert_eq!(original.radius, 0);
    let mut rgba = Vec::new();
    for y in 0..256 {
        for x in 0..256 {
            rgba.extend([x as u8, y as u8, 37, 255]);
        }
    }
    let tile = Tile {
        width: 256,
        height: 256,
        rgba,
    };
    let sample = |id| NumericSample {
        id: Some(id),
        data_version: Some(1343),
    };
    assert_eq!(
        original.color(sample(1), Kind::Grass, Some(&tile)).unwrap(),
        [50, 173, 37]
    );
    assert_eq!(
        original.color(sample(2), Kind::Grass, Some(&tile)).unwrap(),
        [0, 255, 37]
    );
    assert_eq!(
        original
            .color(sample(12), Kind::Foliage, Some(&tile))
            .unwrap(),
        [255, 255, 37]
    );
    assert_eq!(
        original.color(sample(1), Kind::Water, None).unwrap(),
        [255; 3]
    );
    assert!(original
        .color(sample(255), Kind::Grass, Some(&tile))
        .is_err());
    assert!(original.color(sample(1), Kind::Grass, None).is_err());
    assert!(original
        .color(
            NumericSample {
                id: Some(1),
                data_version: Some(2586)
            },
            Kind::Grass,
            Some(&tile)
        )
        .is_err());
    assert!(original
        .color(
            NumericSample {
                id: None,
                data_version: Some(1343)
            },
            Kind::Grass,
            Some(&tile)
        )
        .is_err());
    let mut resources = Resources::default();
    resources.begin_legacy();
    resources.grass_colormap = Some(tile.clone());
    resources.foliage_colormap = Some(tile);
    let bank = TextureBank {
        tints: Arc::new(resources),
        ..Default::default()
    };
    let volume = Volume {
        size: [16, 1, 16],
        cells: vec![1; 256],
        data_version: Some(1343),
    };
    let source = Region::new(None)
        .with_numeric_biomes([0, 64, 0], &volume)
        .unwrap();
    let mut context = Context::new(&bank, &source);
    let grass = parse_state("minecraft:grass").unwrap();
    assert_eq!(
        context.block(&grass, 0, Some("minecraft:desert"), (8, 64, 8)),
        [50. / 255., 173. / 255., 37. / 255.]
    );
    let mut changed = volume.clone();
    changed.cells.fill(2);
    let changed = source
        .clone()
        .with_numeric_biomes([0, 64, 0], &changed)
        .unwrap();
    assert!(!source.same_neighborhood(&changed, 0, 0));
    assert_eq!(
        Context::new(&bank, &changed).block(&grass, 0, None, (8, 64, 8)),
        [0., 1., 37. / 255.]
    );
    let json=r#"{"format":1,"source":"测试显式数字表","versions":{"minimum":2586,"maximum":2586,"unversioned":false},"blend_radius":1,"entries":[{"id":70000,"name":"自定义群系","temperature":0.5,"rainfall":0.5,"water_color":255,"grass_color":16711680,"foliage_color":65280}]}"#.as_bytes();
    let mut custom = Resources::default();
    custom.insert_numeric(json).unwrap();
    let bank = TextureBank {
        tints: Arc::new(custom),
        ..Default::default()
    };
    let volume = Volume {
        size: [16, 1, 16],
        cells: vec![70000; 256],
        data_version: Some(2586),
    };
    let source = Region::new(None)
        .with_numeric_biomes([-16, 64, -16], &volume)
        .unwrap();
    let mut context = Context::new(&bank, &source);
    assert_eq!(context.block(&grass, 0, None, (-8, 64, -8)), [1., 0., 0.]);
    assert_eq!(
        context.block(
            &parse_state("minecraft:oak_leaves").unwrap(),
            0,
            None,
            (-8, 64, -8)
        ),
        [0., 1., 0.]
    );
    assert_eq!(
        context.fluid(
            mcedit2_world::fluid_surface::Kind::Water,
            None,
            (-8, 64, -8)
        ),
        [0., 0., 1.]
    );
    assert_eq!(context.block(&grass, 0, None, (-16, 64, -16)), [1., 0., 1.]);
    let broken = String::from_utf8(json.to_vec())
        .unwrap()
        .replace("70000", "1")
        .replace("16711680", "16777216");
    assert!(Profile::parse(broken.as_bytes()).is_err());
    let p = bank.tints.numeric.as_ref().unwrap();
    assert!(p
        .color(
            NumericSample {
                id: Some(70001),
                data_version: Some(2586)
            },
            Kind::Grass,
            None
        )
        .is_err());
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Versions {
    minimum: i32,
    maximum: i32,
    unversioned: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: i32,
    name: String,
    temperature: f32,
    rainfall: f32,
    water_color: u32,
    grass_color: Option<u32>,
    foliage_color: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    format: u32,
    source: String,
    versions: Versions,
    blend_radius: u8,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
pub(super) struct Profile {
    pub source: String,
    versions: Versions,
    pub radius: u8,
    entries: BTreeMap<i32, Entry>,
}
impl Profile {
    pub fn original() -> Arc<Self> {
        static PROFILE: OnceLock<Arc<Profile>> = OnceLock::new();
        Arc::clone(PROFILE.get_or_init(|| {
            Arc::new(
                Self::parse(include_bytes!("../assets/legacy-biomes.json"))
                    .expect("原版 MCEdit2 气候表必须通过迁移验证"),
            )
        }))
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let source: Source =
            serde_json::from_slice(bytes).map_err(|e| format!("数字 biome 颜色表无效：{e}"))?;
        if source.format != 1
            || source.source.trim().is_empty()
            || source.versions.minimum < 0
            || source.versions.minimum > source.versions.maximum
            || source.blend_radius > 7
            || source.entries.is_empty()
        {
            return Err("数字 biome 颜色表的格式、来源、版本、半径或条目数量无效".into());
        }
        let mut entries = BTreeMap::new();
        for entry in source.entries {
            if entry.name.trim().is_empty()
                || !entry.temperature.is_finite()
                || !entry.rainfall.is_finite()
                || !(0.0..=1.0).contains(&entry.rainfall)
                || [
                    Some(entry.water_color),
                    entry.grass_color,
                    entry.foliage_color,
                ]
                .into_iter()
                .flatten()
                .any(|c| c > 0xffffff)
            {
                return Err(format!("数字 biome {} 的名称、气候或 RGB 无效", entry.id));
            }
            let id = entry.id;
            if entries.insert(id, entry).is_some() {
                return Err(format!("数字 biome ID {id} 重复"));
            }
        }
        Ok(Self {
            source: source.source,
            versions: source.versions,
            radius: source.blend_radius,
            entries,
        })
    }
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.source.capacity()
            + self
                .entries
                .values()
                .map(|v| std::mem::size_of::<Entry>() + v.name.capacity() + 64)
                .sum::<usize>()
    }
    pub fn choices(&self) -> impl Iterator<Item = (i32, &str)> {
        self.entries.iter().map(|(&id, v)| (id, v.name.as_str()))
    }
    pub fn color(
        &self,
        sample: NumericSample,
        kind: Kind,
        tile: Option<&Tile>,
    ) -> Result<[u32; 3], String> {
        let version_matches = match sample.data_version {
            Some(v) => (self.versions.minimum..=self.versions.maximum).contains(&v),
            None => self.versions.unversioned,
        };
        if !version_matches {
            return Err(format!(
                "数字 biome 源版本 {:?} 不在颜色表声明范围内",
                sample.data_version
            ));
        }
        let id = sample
            .id
            .ok_or("数字 biome 容器的高度合同未绑定，不能按坐标取色")?;
        let entry = self
            .entries
            .get(&id)
            .ok_or_else(|| format!("颜色表未定义数字 biome {id}"))?;
        let fixed = match kind {
            Kind::Water => Some(entry.water_color),
            Kind::Grass => entry.grass_color,
            Kind::Foliage => entry.foliage_color,
        };
        let color = if let Some(color) = fixed {
            color
        } else {
            let tile = tile.ok_or("数字 biome 取色缺少资源包草 / 叶色图")?;
            if tile.width != 256 || tile.height != 256 || tile.rgba.len() != 256 * 256 * 4 {
                return Err("数字 biome 色图必须为完整 256×256 RGBA".into());
            }
            let temperature = entry.temperature.clamp(0., 1.);
            let humidity = entry.rainfall.clamp(0., 1.) * temperature;
            let x = ((1. - f64::from(temperature)) * 255.) as usize;
            let y = ((1. - f64::from(humidity)) * 255.) as usize;
            let at = (y * 256 + x) * 4;
            u32::from(tile.rgba[at]) << 16
                | u32::from(tile.rgba[at + 1]) << 8
                | u32::from(tile.rgba[at + 2])
        };
        Ok([(color >> 16) & 255, (color >> 8) & 255, color & 255])
    }
}
