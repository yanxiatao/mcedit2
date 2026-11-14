//! 原版方块实体的静态外形，复用 JSON 模型、纹理依赖、绘制和源拾取。
//! 箱子 / 床参照 Amulet java_vanilla_fix；潜影盒核对客户端 ModelPart 与渲染变换。
//! 不包含开合动画或 NBT 图案。
use super::*;
use serde_json::{json, Map};

const COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// 此处只接管原版统一变体指向的实体占位模型。显式 elements（包括空数组）、
/// loader、multipart 或自定义模型路径继续由资源包解释，不能被内建外形覆盖。
fn stub(bank: &ModelBank, block: &str, model: &str) -> bool {
    let Some(variants) = bank
        .files
        .get(&format!("state:minecraft:{block}"))
        .and_then(|v| v.get("variants"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    if variants.len() != 1 {
        return false;
    }
    let Some(choice) = variants
        .get("")
        .or_else(|| bank.legacy.as_ref().and_then(|_| variants.get("normal")))
        .and_then(Value::as_object)
    else {
        return false;
    };
    if choice.len() != 1
        || choice
            .get("model")
            .and_then(Value::as_str)
            .and_then(normalize_model)
            .as_deref()
            != Some(&format!("model:minecraft:{model}"))
    {
        return false;
    }
    let Some(doc) = bank
        .files
        .get(&format!("model:minecraft:{model}"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    doc.keys().all(|k| {
        matches!(
            k.as_str(),
            "textures" | "display" | "gui_light" | "ambientocclusion" | "parent"
        )
    }) && doc.get("parent").is_none_or(|p| {
        matches!(
            p.as_str(),
            Some("builtin/entity" | "minecraft:builtin/entity")
        )
    }) && doc.get("textures").is_none_or(|v| {
        v.as_object()
            .is_some_and(|t| t.keys().all(|k| k == "particle"))
    })
}

impl ModelBank {
    fn record_static_block_entity(&mut self, block: &str) -> Result<(), String> {
        let id = format!("minecraft:{block}");
        let state = self
            .files
            .get(&format!("state:{id}"))
            .ok_or_else(|| format!("静态方块实体缺少已注册的方块状态：{id}"))?
            .clone();
        self.static_block_entities.insert(id, state);
        Ok(())
    }

    pub(crate) fn is_static_block_entity(&self, state: &State) -> bool {
        let state = self.render_state(state);
        self.static_block_entities
            .get(&state.name)
            .is_some_and(|expected| {
                self.files.get(&format!("state:{}", state.name)) == Some(expected)
            })
    }
    pub(crate) fn vanilla_piston_supported(&self, state: &State) -> bool {
        let state = self.render_state(state);
        if state.name == "minecraft:moving_piston" {
            return stub(self, "moving_piston", "moving_piston");
        }
        state.name == "minecraft:piston_extension"
            && self
                .legacy
                .as_ref()
                .is_some_and(|registry| registry.version() == crate::legacy_render::Version::V112)
            && !self.files.contains_key("state:minecraft:piston_extension")
    }
    pub(super) fn is_vanilla_entity_stub(&self, block: &str, model: &str) -> bool {
        stub(self, block, model)
    }
    /// 未核实旧版本只保留诊断外形；资源包自行定义的显式几何仍由原模型管线处理。
    pub(crate) fn unsupported_legacy_skull(&self, state: &State) -> bool {
        self.legacy
            .as_ref()
            .is_some_and(|registry| registry.version() != crate::legacy_render::Version::V112)
            && self.render_state(state).name == "minecraft:skull"
            && (stub(self, "skull", "skull")
                || (!self.files.contains_key("state:minecraft:skull")
                    && !self.files.contains_key("model:minecraft:skull")))
    }
    /// 只接管原版头颅占位；旧数字状态的方向和版本均来自已绑定的客户端注册表。
    pub(crate) fn vanilla_skull_supported(&self, state: &State) -> bool {
        let state = self.render_state(state);
        let Some(block) = state.name.strip_prefix("minecraft:") else {
            return false;
        };
        let known = match block {
            "skull" => self
                .legacy
                .as_ref()
                .is_some_and(|registry| registry.version() == crate::legacy_render::Version::V112),
            "skeleton_skull"
            | "skeleton_wall_skull"
            | "wither_skeleton_skull"
            | "wither_skeleton_wall_skull"
            | "zombie_head"
            | "zombie_wall_head"
            | "player_head"
            | "player_wall_head"
            | "creeper_head"
            | "creeper_wall_head"
            | "dragon_head"
            | "dragon_wall_head"
            | "piglin_head"
            | "piglin_wall_head" => true,
            _ => false,
        };
        known && stub(self, block, "skull")
    }
    pub(crate) fn vanilla_sign_supported(&self, state: &State) -> bool {
        let state = self.render_state(state);
        let Some(block) = state.name.strip_prefix("minecraft:") else {
            return false;
        };
        if matches!(block, "standing_sign" | "wall_sign") {
            return self
                .legacy
                .as_ref()
                .is_some_and(|registry| registry.version() == crate::legacy_render::Version::V112)
                && stub(self, block, "sign");
        }
        for wood in mcedit2_world::sign::WOODS {
            if block == format!("{wood}_sign") || block == format!("{wood}_wall_sign") {
                return stub(self, block, &format!("{wood}_sign"));
            }
            if block == format!("{wood}_hanging_sign")
                || block == format!("{wood}_wall_hanging_sign")
            {
                return stub(self, block, &format!("{wood}_hanging_sign"));
            }
        }
        false
    }
    /// 动态旗帜只替代原版占位模型，数字状态必须先经过绑定版本的注册表。
    pub(crate) fn vanilla_banner_supported(&self, state: &State) -> bool {
        let state = self.render_state(state);
        let Some(block) = state.name.strip_prefix("minecraft:") else {
            return false;
        };
        let known = if matches!(block, "standing_banner" | "wall_banner") {
            self.legacy.is_some()
        } else {
            block
                .strip_suffix("_wall_banner")
                .or_else(|| block.strip_suffix("_banner"))
                .is_some_and(|color| COLORS.contains(&color))
        };
        known && stub(self, block, "banner")
    }
    /// 仅验证本次内建旧箱子实际接管的实体图布局；资源包自定义模型自行定义 UV。
    pub(crate) fn legacy_chest_sheet_ratio(&self, id: &str) -> Option<(u32, u32)> {
        let sheet = id.strip_prefix("minecraft:entity/chest/")?;
        let (block, double) = match sheet {
            "normal" => ("chest", false),
            "normal_double" => ("chest", true),
            "trapped" => ("trapped_chest", false),
            "trapped_double" => ("trapped_chest", true),
            "ender" => ("ender_chest", false),
            _ => return None,
        };
        self.legacy_chests
            .contains(&format!("minecraft:{block}"))
            .then_some((if double { 2 } else { 1 }, 1))
    }
    fn builtin_document(&mut self, path: &str, value: Value) -> Result<(), String> {
        if !self.insert(
            path,
            &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
        )? {
            return Err(format!("内建静态模型路径无效：{path}"));
        }
        Ok(())
    }
    /// 为原版静态方块实体建立具体模型；旧版须有版本绑定，现代箱子须有分半贴图。
    /// 返回接管类别数和未能提供的已知实体模型诊断；不覆盖资源包显式模型。
    pub(crate) fn install_vanilla_specials(
        &mut self,
        modern_chests: bool,
    ) -> Result<(usize, Vec<String>), String> {
        let mut installed = 0;
        let mut warnings = Vec::new();
        if let Some(registry) = self.legacy.as_ref() {
            if registry.version() == crate::legacy_render::Version::V112 {
                // 旧客户端用实体模型渲染告示牌，源尺寸 / UV / 变换已由准确 1.12.2 静态类核实。
                let signs = ["standing_sign", "wall_sign"]
                    .into_iter()
                    .filter(|block| {
                        !self.files.contains_key(&format!("state:minecraft:{block}"))
                            && !self.files.contains_key(&format!("model:minecraft:{block}"))
                    })
                    .collect::<Vec<_>>();
                if !signs.is_empty() && !self.files.contains_key("model:minecraft:sign") {
                    self.builtin_document(
                        "assets/minecraft/models/block/sign.json",
                        json!({"textures":{"particle":"minecraft:blocks/planks_oak"}}),
                    )?;
                    for block in signs {
                        self.builtin_document(
                            &format!("assets/minecraft/blockstates/{block}.json"),
                            json!({"variants":{"":{"model":"minecraft:block/sign"}}}),
                        )?;
                    }
                }
                // 1.12 头颅只有实体模型；真实资源未定义对应 JSON 时才建立占位。
                if !self.files.contains_key("state:minecraft:skull")
                    && !self.files.contains_key("model:minecraft:skull")
                {
                    self.builtin_document("assets/minecraft/models/block/skull.json", json!({}))?;
                    self.builtin_document(
                        "assets/minecraft/blockstates/skull.json",
                        json!({"variants":{"":{"model":"minecraft:block/skull"}}}),
                    )?;
                }
            } else {
                warnings.push("当前旧客户端版本的头颅渲染器尚未核实，保留缺失材质诊断，不套用 1.12 或现代变换。".into());
            }
        }
        // 旧版客户端没有旗帜 blockstate / block model，真实外形全部来自实体层。
        // 仅在版本绑定且资源未自行提供模型时建立空占位，避免额外的缺失材质方块遮挡布面。
        if self.legacy.is_some() && !self.files.contains_key("model:minecraft:banner") {
            let blocks = ["standing_banner", "wall_banner"]
                .into_iter()
                .filter(|block| {
                    !self.files.contains_key(&format!("state:minecraft:{block}"))
                        && !self.files.contains_key(&format!("model:minecraft:{block}"))
                })
                .collect::<Vec<_>>();
            if !blocks.is_empty() {
                self.builtin_document("assets/minecraft/models/block/banner.json", json!({}))?;
                for block in blocks {
                    self.builtin_document(
                        &format!("assets/minecraft/blockstates/{block}.json"),
                        json!({"variants":{"":{"model":"minecraft:block/banner"}}}),
                    )?;
                }
            }
        }
        if !modern_chests && self.legacy.is_none() {
            for (block, model) in [
                ("chest", "chest"),
                ("trapped_chest", "chest"),
                ("ender_chest", "ender_chest"),
            ] {
                if stub(self, block, model) {
                    warnings.push(format!("minecraft:{block} 未识别现代分半箱子贴图，且没有已核实的旧版资源绑定；不能仅凭旧实体贴图尺寸猜测布局，未生成静态外形。"));
                }
            }
        }
        if self.legacy.is_some() {
            for (block, sheet, source_model) in [
                ("chest", "normal", "chest"),
                ("trapped_chest", "trapped", "chest"),
                ("ender_chest", "ender", "ender_chest"),
            ] {
                // 已核实 1.12.2 客户端没有这些 blockstate / block model 文件；
                // 旧游戏由实体渲染器负责。资源包一旦显式声明任何一项便保留其解释权。
                if !stub(self, block, source_model)
                    && (self.files.contains_key(&format!("state:minecraft:{block}"))
                        || self
                            .files
                            .contains_key(&format!("model:minecraft:{source_model}")))
                {
                    continue;
                }
                let mut variants = Map::new();
                let kinds: &[&str] = if block == "ender_chest" {
                    &["single"]
                } else {
                    &["single", "left", "right", "invalid"]
                };
                for &kind in kinds {
                    let name = format!("builtin/legacy_chest/{block}_{kind}");
                    if self.files.contains_key(&format!("model:mcedit2:{name}")) {
                        return Err(format!("资源包占用了保留的内建模型标识 mcedit2:{name}"));
                    }
                    let texture = if kind == "invalid" {
                        crate::textures::MISSING_TEXTURE_ID.into()
                    } else {
                        format!(
                            "minecraft:entity/chest/{sheet}{}",
                            if kind == "single" { "" } else { "_double" }
                        )
                    };
                    self.builtin_document(
                        &format!("assets/mcedit2/models/{name}.json"),
                        crate::layer_renderer::builtin_element_document(
                            &format!(
                                "legacy_chest/{}",
                                if kind == "invalid" { "single" } else { kind }
                            ),
                            &json!({"texture":texture}),
                        )?,
                    )?;
                    for (dir, y) in [("south", 0), ("west", 90), ("north", 180), ("east", 270)] {
                        let choice = json!({"model":format!("mcedit2:{name}"),"y":y});
                        variants.insert(format!("facing={dir},type={kind}"), choice.clone());
                        if kind == "single" {
                            variants.insert(format!("facing={dir}"), choice);
                        }
                    }
                }
                self.builtin_document(
                    &format!("assets/minecraft/blockstates/{block}.json"),
                    json!({"variants":variants}),
                )?;
                self.legacy_chests.insert(format!("minecraft:{block}"));
                self.record_static_block_entity(block)?;
                installed += 1;
            }
        }
        if modern_chests && self.legacy.is_none() {
            for (block, sheet, source_model) in [
                ("chest", "normal", "chest"),
                ("trapped_chest", "trapped", "chest"),
                ("ender_chest", "ender", "ender_chest"),
            ] {
                if !stub(self, block, source_model) {
                    continue;
                }
                let mut variants = Map::new();
                let kinds: &[&str] = if block == "ender_chest" {
                    &["single"]
                } else {
                    &["single", "left", "right"]
                };
                for &kind in kinds {
                    let name = format!("builtin/chest/{block}_{kind}");
                    let texture = format!(
                        "minecraft:entity/chest/{sheet}{}",
                        if kind == "single" {
                            String::new()
                        } else {
                            format!("_{kind}")
                        }
                    );
                    let path = format!("assets/mcedit2/models/{name}.json");
                    if self.files.contains_key(&format!("model:mcedit2:{name}")) {
                        return Err(format!("资源包占用了保留的内建模型标识 mcedit2:{name}"));
                    }
                    self.builtin_document(
                        &path,
                        crate::layer_renderer::builtin_element_document(
                            &format!("chest/{kind}"),
                            &json!({"texture":texture}),
                        )?,
                    )?;
                    for (dir, y) in [("south", 0), ("west", 90), ("north", 180), ("east", 270)] {
                        let key = if block == "ender_chest" {
                            format!("facing={dir}")
                        } else {
                            format!("facing={dir},type={kind}")
                        };
                        variants.insert(key, json!({"model":format!("mcedit2:{name}"),"y":y}));
                    }
                }
                self.builtin_document(
                    &format!("assets/minecraft/blockstates/{block}.json"),
                    json!({"variants":variants}),
                )?;
                self.record_static_block_entity(block)?;
                installed += 1;
            }
        }
        for color in COLORS {
            let block = format!("{color}_bed");
            if !stub(self, &block, "bed") {
                continue;
            }
            let mut variants = Map::new();
            for part in ["head", "foot"] {
                let name = format!("builtin/bed/{color}_{part}");
                if self.files.contains_key(&format!("model:mcedit2:{name}")) {
                    return Err(format!("资源包占用了保留的内建模型标识 mcedit2:{name}"));
                }
                self.builtin_document(
                    &format!("assets/mcedit2/models/{name}.json"),
                    crate::layer_renderer::builtin_element_document(
                        &format!("bed/{part}"),
                        &json!({"texture":format!("minecraft:entity/bed/{color}")}),
                    )?,
                )?;
                for (dir, y) in [("north", 0), ("east", 90), ("south", 180), ("west", 270)] {
                    variants.insert(
                        format!("part={part},facing={dir}"),
                        json!({"model":format!("mcedit2:{name}"),"y":y}),
                    );
                }
            }
            self.builtin_document(
                &format!("assets/minecraft/blockstates/{block}.json"),
                json!({"variants":variants}),
            )?;
            self.record_static_block_entity(&block)?;
            installed += 1;
        }
        for color in std::iter::once("").chain(COLORS) {
            let (block, sheet) = if color.is_empty() {
                ("shulker_box".to_string(), "shulker".to_string())
            } else {
                (format!("{color}_shulker_box"), format!("shulker_{color}"))
            };
            if !stub(self, &block, &block) {
                continue;
            }
            let name = format!("builtin/shulker/{block}");
            if self.files.contains_key(&format!("model:mcedit2:{name}")) {
                return Err(format!("资源包占用了保留的内建模型标识 mcedit2:{name}"));
            }
            self.builtin_document(
                &format!("assets/mcedit2/models/{name}.json"),
                crate::layer_renderer::builtin_element_document(
                    "shulker",
                    &json!({"texture":format!("minecraft:entity/shulker/{sheet}")}),
                )?,
            )?;
            let mut variants = Map::new();
            // Direction.getRotation 的等价中心旋转；north 还需 Y=180，
            // 只使用 X=90 虽然开口朝北，却会使实体贴图方向错误。
            for (dir, x, y) in [
                ("up", 0, 0),
                ("down", 180, 0),
                ("south", 270, 0),
                ("north", 270, 180),
                ("west", 270, 90),
                ("east", 270, 270),
            ] {
                variants.insert(
                    format!("facing={dir}"),
                    json!({"model":format!("mcedit2:{name}"),"x":x,"y":y}),
                );
            }
            self.builtin_document(
                &format!("assets/minecraft/blockstates/{block}.json"),
                json!({"variants":variants}),
            )?;
            self.record_static_block_entity(&block)?;
            installed += 1;
        }
        Ok((installed, warnings))
    }
}
