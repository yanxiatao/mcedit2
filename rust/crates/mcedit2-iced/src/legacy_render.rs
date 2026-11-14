//! 复用原版 BlockDumper 的精确 ID/metadata/resourcePath/resourceVariant 绑定。
//! 渲染绑定不进入编辑 Session，不推断模组 ID，也不把数字存档转换为命名存档。
use mcedit2_world::{editing::parse_state, sections::State};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Version {
    V18,
    V19,
    V110,
    V111,
    V112,
}
impl Version {
    pub(crate) fn from_release(version: &str) -> Option<Self> {
        let parts = version.split('.').collect::<Vec<_>>();
        if !(2..=3).contains(&parts.len())
            || parts
                .iter()
                .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
            || parts[0] != "1"
        {
            return None;
        }
        match parts[1] {
            "8" => Some(Self::V18),
            "9" => Some(Self::V19),
            "10" => Some(Self::V110),
            "11" => Some(Self::V111),
            "12" => Some(Self::V112),
            _ => None,
        }
    }
    pub(crate) fn from_client_path(path: &Path) -> Option<Self> {
        if path.extension()?.to_str()? != "jar" {
            return None;
        }
        let stem = path.file_stem()?.to_str()?;
        let version = stem
            .strip_prefix("minecraft-")
            .and_then(|s| s.strip_suffix("-client"))
            .unwrap_or(stem);
        Self::from_release(version)
    }
    pub(crate) fn registry(self) -> Arc<Registry> {
        static V18: OnceLock<Arc<Registry>> = OnceLock::new();
        static V19: OnceLock<Arc<Registry>> = OnceLock::new();
        static V110: OnceLock<Arc<Registry>> = OnceLock::new();
        static V111: OnceLock<Arc<Registry>> = OnceLock::new();
        static V112: OnceLock<Arc<Registry>> = OnceLock::new();
        let (slot, json) = match self {
            Self::V18 => (
                &V18,
                mcedit2_world::legacy_assets::mapping("1.8").expect("缺少内置旧版方块映射"),
            ),
            Self::V19 => (
                &V19,
                mcedit2_world::legacy_assets::mapping("1.9").expect("缺少内置旧版方块映射"),
            ),
            Self::V110 => (
                &V110,
                mcedit2_world::legacy_assets::mapping("1.10").expect("缺少内置旧版方块映射"),
            ),
            Self::V111 => (
                &V111,
                mcedit2_world::legacy_assets::mapping("1.11").expect("缺少内置旧版方块映射"),
            ),
            Self::V112 => (
                &V112,
                mcedit2_world::legacy_assets::mapping("1.12").expect("缺少内置旧版方块映射"),
            ),
        };
        Arc::clone(slot.get_or_init(|| {
            Arc::new(Registry::parse(json).expect("原版静态方块映射必须通过生成与回归验证"))
        }))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Binding {
    pub state: State,
    pub source_state: State,
    pub variant: Option<String>,
}
#[derive(Debug)]
pub(crate) struct Registry {
    version: Version,
    bindings: BTreeMap<(u16, u8), Binding>,
    names: BTreeMap<String, u16>,
}
type SourceEntry = (u16, u8, Option<String>, Option<String>, String, String);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    source: String,
    version: String,
    entries: Vec<SourceEntry>,
}
impl Registry {
    fn parse(json: &str) -> Result<Self, String> {
        let source: Source = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let version = Version::from_release(&source.version);
        if source.source.len() != 40 || version.is_none() {
            return Err("原版方块映射来源无效".into());
        }
        let mut bindings = BTreeMap::new();
        let mut names = BTreeMap::new();
        for (id, meta, path, variant, internal_name, original_state) in source.entries {
            let internal = parse_state(&original_state)
                .map_err(|e| format!("{id}:{meta} {original_state}: {e}"))?;
            if internal.name != internal_name || meta > 15 {
                return Err("原版方块映射身份不符".into());
            }
            if names
                .insert(internal_name, id)
                .is_some_and(|previous| previous != id)
            {
                return Err("旧方块注册名绑定了多个数字 ID".into());
            }
            let state = if let Some(path) = path {
                let mut state = State {
                    name: format!("minecraft:{path}"),
                    properties: Vec::new(),
                };
                if let Some(variant) = variant.as_ref().filter(|v| v.contains('=')) {
                    state = parse_state(&format!("{}[{variant}]", state.name))
                        .map_err(|e| format!("{id}:{meta} {}[{variant}]: {e}", state.name))?;
                }
                state
            } else {
                let mut state = internal.clone();
                // 原版流体渲染器共用静止/流动纹理；这是源注册表中的两个原版 ID。
                match state.name.as_str() {
                    "minecraft:flowing_water" => state.name = "minecraft:water".into(),
                    "minecraft:flowing_lava" => state.name = "minecraft:lava".into(),
                    _ => (),
                }
                state
            };
            if bindings
                .insert(
                    (id, meta),
                    Binding {
                        state,
                        source_state: internal,
                        variant,
                    },
                )
                .is_some()
            {
                return Err("重复原版数字状态".into());
            }
        }
        // 原 BlockDumper 按状态去重；准确客户端已核实三类箱子的全部 nibble 解码。
        // 与编辑旋转共用方向合同，模型及资源 variant 仍取实际原始绑定。
        if version == Some(Version::V112) {
            for id in [54, 130, 146] {
                for meta in 0..16 {
                    if bindings.contains_key(&(id, meta)) {
                        continue;
                    }
                    let facing = mcedit2_world::legacy_assets::chest_facing_1_12(meta)
                        .expect("已限定 nibble");
                    let binding = bindings
                        .iter()
                        .find(|((block, _), b)| {
                            *block == id
                                && b.state
                                    .properties
                                    .iter()
                                    .any(|(key, value)| key == "facing" && value == facing)
                        })
                        .ok_or("旧箱子数字别名缺少已核对的方向基准")?
                        .1
                        .clone();
                    if bindings.insert((id, meta), binding).is_some() {
                        return Err("旧箱子数字别名与原始绑定冲突".into());
                    }
                }
            }
        }
        Ok(Self {
            version: version.ok_or("旧版资源缺少已验证版本")?,
            bindings,
            names,
        })
    }
    pub(crate) fn resolve(&self, state: &State) -> Option<&Binding> {
        let id = state.name.strip_prefix("numeric:")?.parse::<u16>().ok()?;
        let meta = match state.properties.as_slice() {
            [(key, value)] if key == "data" => value.parse::<u8>().ok()?,
            _ => return None,
        };
        self.bindings.get(&(id, meta))
    }
    /// 只有生成来源验证过的版本可选择对应原版渲染规则。
    pub(crate) fn version(&self) -> Version {
        self.version
    }
    /// 方块选择器展示准确数字状态，并用原始注册名搜索，不借用现代模型重命名。
    pub(crate) fn states(&self) -> impl Iterator<Item = (State, &State)> {
        self.bindings.iter().map(|(&(id, data), binding)| {
            (
                State {
                    name: format!("numeric:{id}"),
                    properties: vec![("data".into(), data.to_string())],
                },
                &binding.source_state,
            )
        })
    }
    /// 实体旧 carried 的注册名由准确源目录解析，不借用现代重命名后的模型名。
    pub(crate) fn named_numeric_state(&self, name: &str, meta: u16) -> Result<State, String> {
        let name = if name.contains(':') {
            name.to_owned()
        } else {
            format!("minecraft:{name}")
        };
        let id = self
            .names
            .get(&name)
            .ok_or_else(|| format!("旧资源注册目录没有方块 {name}"))?;
        let meta = u8::try_from(meta).map_err(|_| "旧搬运方块 metadata 超出资源目录范围")?;
        if !self.bindings.contains_key(&(*id, meta)) {
            return Err(format!("旧资源注册目录没有 {name} 的 metadata={meta}"));
        }
        Ok(State {
            name: format!("numeric:{id}"),
            properties: vec![("data".into(), meta.to_string())],
        })
    }
    /// 缺失方块实体诊断使用实际版本映射中的精确数字状态，不猜数字 ID 或 metadata 别名。
    pub(crate) fn block_visual_states(&self) -> impl Iterator<Item = ((u16, u8), &State)> {
        self.bindings.iter().filter_map(|(key, binding)| {
            matches!(
                binding.state.name.as_str(),
                "minecraft:standing_banner"
                    | "minecraft:wall_banner"
                    | "minecraft:skull"
                    | "minecraft:standing_sign"
                    | "minecraft:wall_sign"
                    | "minecraft:piston_extension"
            )
            .then_some((*key, &binding.state))
        })
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_original_tables_load_without_unknown_id_inference() {
        for version in [
            Version::V18,
            Version::V19,
            Version::V110,
            Version::V111,
            Version::V112,
        ] {
            let registry = version.registry();
            let granite = registry
                .resolve(&parse_state("numeric:1[data=1]").unwrap())
                .unwrap();
            assert_eq!(granite.state.name, "minecraft:granite");
            assert!(granite.state.properties.is_empty());
            let stairs = registry
                .resolve(&parse_state("numeric:53[data=4]").unwrap())
                .unwrap();
            assert!(stairs
                .state
                .properties
                .contains(&("half".into(), "top".into())));
            assert!(registry
                .resolve(&parse_state("numeric:301[data=0]").unwrap())
                .is_none());
        }
        assert_eq!(
            Version::from_client_path(Path::new("minecraft-1.12.2-client.jar")),
            Some(Version::V112)
        );
        assert_eq!(
            Version::from_client_path(Path::new("1.8.9.jar")),
            Some(Version::V18)
        );
        assert_eq!(Version::from_client_path(Path::new("unknown.jar")), None);
        let registry = Version::V112.registry();
        let directions = [
            "north", "north", "north", "south", "west", "east", "north", "north", "north", "south",
            "west", "east", "north", "north", "north", "south",
        ];
        for id in [54, 130, 146] {
            for (meta, facing) in directions.into_iter().enumerate() {
                let binding = registry
                    .resolve(&parse_state(&format!("numeric:{id}[data={meta}]")).unwrap())
                    .unwrap();
                assert!(binding
                    .state
                    .properties
                    .contains(&("facing".into(), facing.into())));
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Models {
            all_original_tables_load_without_unknown_id_inference();
        }
    }
}
