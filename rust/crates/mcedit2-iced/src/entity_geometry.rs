//! 原版静态生物模型：保留主要外观、位置、朝向和尺寸，不以精细动画姿态为显示门槛。
//! 后台编译、源实体归属及与地形同一世代的 GPU 提交仍使用实际模型来源。
use crate::{
    entity_visibility::Visibility,
    scene_layers::{MarkerRef, Snapshot},
    spectator::{MeshChunk, StreamMesh},
    textures::{RuntimeTexture, TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
    Message,
};
use iced::Task;
use mcedit2_world::{
    entity_catalog::EntityVisual,
    entity_model::{Catalog, PartPose},
    nbt_document::Value,
    scene_annotations::{BlockVisual, Kind, Marker},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};
#[path = "entity_allay.rs"]
mod allay;
#[path = "entity_animals.rs"]
pub(crate) mod animals;
#[path = "entity_bats_squids.rs"]
pub(crate) mod bats_squids;
#[path = "entity_bees_phantoms.rs"]
pub(crate) mod bees_phantoms;
#[path = "entity_cats.rs"]
pub(crate) mod cats;
#[path = "entity_creatures.rs"]
mod creatures;
#[path = "entity_enderman.rs"]
mod enderman;
#[path = "entity_fish.rs"]
mod fish;
#[path = "entity_foxes_goats.rs"]
mod foxes_goats;
#[path = "entity_golems.rs"]
pub(crate) mod golems;
#[path = "entity_guardians.rs"]
pub(crate) mod guardians;
#[path = "entity_monsters.rs"]
mod monsters;
#[path = "entity_nether_beasts.rs"]
pub(crate) mod nether_beasts;
#[path = "entity_pandas_ravagers.rs"]
pub(crate) mod pandas_ravagers;
#[path = "entity_parrots.rs"]
pub(crate) mod parrots;
#[path = "entity_passives.rs"]
pub(crate) mod passives;
#[path = "entity_piglins.rs"]
mod piglins;
#[path = "entity_rig.rs"]
mod rig;
#[path = "entity_static_mobs.rs"]
pub(crate) mod static_mobs;
#[path = "entity_swimmers.rs"]
mod swimmers;
pub(crate) use rig::Rig;
#[path = "entity_eyes.rs"]
mod eyes;
#[path = "entity_illagers.rs"]
mod illagers;
#[path = "entity_large_mobs.rs"]
mod large_mobs;
#[path = "entity_npc.rs"]
mod npc;
#[path = "entity_objects.rs"]
pub(crate) mod objects;
#[path = "entity_scene.rs"]
mod scene;
#[path = "entity_vehicles.rs"]
pub(crate) mod vehicles;
#[path = "entity_vex.rs"]
mod vex;
#[path = "entity_villagers.rs"]
mod villagers;
#[path = "entity_warden.rs"]
mod warden;
#[path = "entity_witch.rs"]
mod witch;
#[path = "entity_wither.rs"]
mod wither;
type SkinResults = BTreeMap<[u8; 16], Result<Arc<crate::player_skin::Appearance>, String>>;
type HeadResults =
    BTreeMap<crate::skull_skin::Key, Result<Arc<crate::skull_skin::Appearance>, String>>;

/// 完成的运行时页按资源基底及实际像素复用；弱引用不延长已离开场景的图集寿命。
pub(crate) fn append_runtime_cached(
    base: &Arc<TextureAtlas>,
    textures: BTreeMap<String, Arc<RuntimeTexture>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<TextureAtlas>, String> {
    use sha2::{Digest, Sha256};
    if cancelled() {
        return Err("运行时材质准备已取消".into());
    }
    if textures.is_empty() {
        return Ok(Arc::clone(base));
    }
    let mut digest = Sha256::new();
    for (id, texture) in &textures {
        if cancelled() {
            return Err("运行时材质准备已取消".into());
        }
        if id != &texture.id {
            return Err("运行时材质索引与源 ID 不一致".into());
        }
        digest.update((id.len() as u64).to_le_bytes());
        digest.update(id.as_bytes());
        digest.update(texture.width.to_le_bytes());
        digest.update(texture.height.to_le_bytes());
        digest.update([u8::from(texture.sampler.is_some())]);
        if let Some(sampler) = texture.sampler {
            digest.update(sampler.flags(false).to_le_bytes());
        }
        for pixels in texture.rgba.chunks(64 * 1024) {
            if cancelled() {
                return Err("运行时材质准备已取消".into());
            }
            digest.update(pixels);
        }
    }
    let digest: [u8; 32] = digest.finalize().into();
    type Entry = (Weak<TextureAtlas>, [u8; 32], Weak<TextureAtlas>);
    static CACHE: OnceLock<Mutex<VecDeque<Entry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(VecDeque::new()));
    {
        let mut entries = cache.lock().expect("运行时图集缓存锁异常");
        entries.retain(|(base, _, atlas)| base.strong_count() > 0 && atlas.strong_count() > 0);
        if let Some(index) = entries
            .iter()
            .position(|(old, hash, _)| old.ptr_eq(&Arc::downgrade(base)) && hash == &digest)
        {
            if let Some(entry) = entries.remove(index) {
                if let Some(atlas) = entry.2.upgrade() {
                    entries.push_back(entry);
                    return Ok(atlas);
                }
            }
        }
    }
    let atlas = Arc::clone(
        &crate::textures::runtime_atlas::reconcile(Arc::clone(base), textures, None, cancelled)?
            .0
            .atlas,
    );
    if cancelled() {
        return Err("运行时材质准备已取消".into());
    }
    let mut entries = cache.lock().expect("运行时图集缓存锁异常");
    while entries.len() >= 8 {
        entries.pop_front();
    }
    entries.push_back((Arc::downgrade(base), digest, Arc::downgrade(&atlas)));
    Ok(atlas)
}
fn player_uuid(marker: &mcedit2_world::scene_annotations::Marker) -> Option<[u8; 16]> {
    let Value::IntArray(words) = marker
        .visual
        .as_ref()?
        .appearance
        .get("mcedit2:player_uuid")?
    else {
        return None;
    };
    if words.len() != 4 {
        return None;
    }
    let mut out = [0; 16];
    for (i, w) in words.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    Some(out)
}

// 只将当前资源源中确实存在的内建依赖加入图集；旧版缺少的皮肤在实体实例处报诊断。
pub(crate) const TEXTURES: &[&str] = &[
    crate::entity_fire::TEXTURES[0],
    crate::entity_fire::TEXTURES[1],
    crate::entity_fire::TEXTURES[2],
    crate::entity_fire::TEXTURES[3],
    eyes::TEXTURE,
    crate::experience_geometry::TEXTURE,
    "minecraft:entity/zombie/zombie",
    "minecraft:entity/zombie/husk",
    "minecraft:entity/zombie/drowned",
    "minecraft:entity/zombie/drowned_outer_layer",
    "minecraft:entity/slime/slime",
    "minecraft:entity/slime/magmacube",
    "minecraft:entity/ghast/ghast",
    "minecraft:entity/blaze",
    "minecraft:entity/silverfish",
    "minecraft:entity/endermite",
    "minecraft:entity/enderman/enderman",
    eyes::ENDERMAN_TEXTURE,
    "minecraft:entity/skeleton/skeleton",
    "minecraft:entity/skeleton/stray",
    "minecraft:entity/skeleton/stray_overlay",
    "minecraft:entity/skeleton/wither_skeleton",
    "minecraft:entity/skeleton/bogged",
    "minecraft:entity/skeleton/bogged_overlay",
    "minecraft:entity/spider/spider",
    "minecraft:entity/spider/cave_spider",
    "minecraft:entity/creeper/creeper",
    "minecraft:entity/cow/cow",
    "minecraft:entity/cow/mooshroom",
    "minecraft:entity/cow/red_mooshroom",
    "minecraft:entity/cow/brown_mooshroom",
    "minecraft:entity/enderdragon/dragon",
    "minecraft:entity/enderdragon/dragon_eyes",
    "minecraft:entity/enderdragon/dragon_fireball",
    "minecraft:entity/projectiles/wind_charge",
    "minecraft:entity/fishing_hook",
    "minecraft:entity/pig/pig",
    "minecraft:entity/pig/pig_saddle",
    "minecraft:entity/chicken",
    "minecraft:entity/bat",
    "minecraft:entity/bee/bee",
    "minecraft:entity/bee/bee_nectar",
    "minecraft:entity/bee/bee_angry",
    "minecraft:entity/bee/bee_angry_nectar",
    "minecraft:entity/phantom",
    eyes::PHANTOM_TEXTURE,
    "minecraft:entity/dolphin",
    "minecraft:entity/turtle/big_sea_turtle",
    "minecraft:entity/fox/fox",
    "minecraft:entity/fox/fox_sleep",
    "minecraft:entity/fox/snow_fox",
    "minecraft:entity/fox/snow_fox_sleep",
    "minecraft:entity/goat/goat",
    "minecraft:entity/panda/panda",
    "minecraft:entity/panda/lazy_panda",
    "minecraft:entity/panda/worried_panda",
    "minecraft:entity/panda/playful_panda",
    "minecraft:entity/panda/brown_panda",
    "minecraft:entity/panda/weak_panda",
    "minecraft:entity/panda/aggressive_panda",
    "minecraft:entity/illager/ravager",
    "minecraft:entity/hoglin/hoglin",
    "minecraft:entity/hoglin/zoglin",
    "minecraft:entity/strider/strider",
    "minecraft:entity/strider/strider_saddle",
    "minecraft:entity/guardian",
    "minecraft:entity/guardian_elder",
    "minecraft:entity/wither/wither",
    "minecraft:entity/wither/wither_invulnerable",
    "minecraft:entity/wither/wither_armor",
    "minecraft:entity/warden/warden",
    "minecraft:entity/warden/warden_bioluminescent_layer",
    "minecraft:entity/warden/warden_pulsating_spots_1",
    "minecraft:entity/warden/warden_pulsating_spots_2",
    "minecraft:entity/warden/warden_heart",
    "minecraft:entity/camel/camel",
    "minecraft:entity/sniffer/sniffer",
    "minecraft:entity/armadillo",
    "minecraft:entity/axolotl/axolotl_lucy",
    "minecraft:entity/axolotl/axolotl_wild",
    "minecraft:entity/axolotl/axolotl_gold",
    "minecraft:entity/axolotl/axolotl_cyan",
    "minecraft:entity/axolotl/axolotl_blue",
    "minecraft:entity/tadpole/tadpole",
    "minecraft:entity/breeze/breeze",
    "minecraft:entity/breeze/breeze_wind",
    "minecraft:entity/breeze/breeze_eyes",
    "minecraft:entity/minecart",
    "minecraft:entity/projectiles/arrow",
    "minecraft:entity/projectiles/spectral_arrow",
    "minecraft:entity/trident",
    "minecraft:entity/shulker/spark",
    "minecraft:entity/llama/spit",
    "minecraft:entity/illager/fangs",
    "minecraft:entity/illager/evoker_fangs",
    "minecraft:entity/lead_knot",
    "minecraft:entity/endercrystal/endercrystal",
    "minecraft:entity/end_crystal/end_crystal",
    "minecraft:entity/boat/boat_oak",
    "minecraft:entity/boat/boat_spruce",
    "minecraft:entity/boat/boat_birch",
    "minecraft:entity/boat/boat_jungle",
    "minecraft:entity/boat/boat_acacia",
    "minecraft:entity/boat/boat_darkoak",
    "minecraft:entity/boat/oak",
    "minecraft:entity/boat/spruce",
    "minecraft:entity/boat/birch",
    "minecraft:entity/boat/jungle",
    "minecraft:entity/boat/acacia",
    "minecraft:entity/boat/dark_oak",
    "minecraft:entity/boat/mangrove",
    "minecraft:entity/boat/cherry",
    "minecraft:entity/boat/bamboo",
    "minecraft:entity/chest_boat/oak",
    "minecraft:entity/chest_boat/spruce",
    "minecraft:entity/chest_boat/birch",
    "minecraft:entity/chest_boat/jungle",
    "minecraft:entity/chest_boat/acacia",
    "minecraft:entity/chest_boat/dark_oak",
    "minecraft:entity/chest_boat/mangrove",
    "minecraft:entity/chest_boat/cherry",
    "minecraft:entity/chest_boat/bamboo",
    "minecraft:entity/parrot/parrot_red_blue",
    "minecraft:entity/parrot/parrot_blue",
    "minecraft:entity/parrot/parrot_green",
    "minecraft:entity/parrot/parrot_yellow_blue",
    "minecraft:entity/parrot/parrot_grey",
    "minecraft:entity/squid",
    "minecraft:entity/squid/squid",
    "minecraft:entity/squid/glow_squid",
    "minecraft:entity/bear/polarbear",
    "minecraft:entity/iron_golem",
    "minecraft:entity/iron_golem/iron_golem",
    "minecraft:entity/iron_golem/iron_golem_crackiness_low",
    "minecraft:entity/iron_golem/iron_golem_crackiness_medium",
    "minecraft:entity/iron_golem/iron_golem_crackiness_high",
    "minecraft:entity/snowman",
    "minecraft:entity/snow_golem",
    "minecraft:entity/rabbit/brown",
    "minecraft:entity/rabbit/white",
    "minecraft:entity/rabbit/black",
    "minecraft:entity/rabbit/white_splotched",
    "minecraft:entity/rabbit/gold",
    "minecraft:entity/rabbit/salt",
    "minecraft:entity/rabbit/caerbannog",
    "minecraft:entity/rabbit/toast",
    "minecraft:entity/frog/temperate_frog",
    "minecraft:entity/frog/warm_frog",
    "minecraft:entity/frog/cold_frog",
    "minecraft:entity/sheep/sheep",
    "minecraft:entity/sheep/sheep_fur",
    "minecraft:entity/villager/villager",
    "minecraft:entity/wandering_trader",
    "minecraft:entity/witch",
    "minecraft:entity/illager/evoker",
    "minecraft:entity/illager/vindicator",
    "minecraft:entity/illager/illusioner",
    "minecraft:entity/illager/illusionist",
    "minecraft:entity/illager/pillager",
    "minecraft:entity/illager/vex",
    "minecraft:entity/allay/allay",
    "minecraft:entity/zombie_villager/zombie_villager",
    "minecraft:entity/zombie_villager/zombie_farmer",
    "minecraft:entity/zombie_villager/zombie_librarian",
    "minecraft:entity/zombie_villager/zombie_priest",
    "minecraft:entity/zombie_villager/zombie_smith",
    "minecraft:entity/zombie_villager/zombie_butcher",
    "minecraft:entity/armorstand/wood",
    "minecraft:entity/piglin/zombified_piglin",
    "minecraft:entity/piglin/piglin",
    "minecraft:entity/piglin/piglin_brute",
    "minecraft:entity/shulker/shulker",
    "minecraft:entity/shulker/shulker_white",
    "minecraft:entity/shulker/shulker_orange",
    "minecraft:entity/shulker/shulker_magenta",
    "minecraft:entity/shulker/shulker_light_blue",
    "minecraft:entity/shulker/shulker_yellow",
    "minecraft:entity/shulker/shulker_lime",
    "minecraft:entity/shulker/shulker_pink",
    "minecraft:entity/shulker/shulker_gray",
    "minecraft:entity/shulker/shulker_light_gray",
    "minecraft:entity/shulker/shulker_cyan",
    "minecraft:entity/shulker/shulker_purple",
    "minecraft:entity/shulker/shulker_blue",
    "minecraft:entity/shulker/shulker_brown",
    "minecraft:entity/shulker/shulker_green",
    "minecraft:entity/shulker/shulker_red",
    "minecraft:entity/shulker/shulker_black",
    "minecraft:entity/player/slim/alex",
    "minecraft:entity/player/slim/ari",
    "minecraft:entity/player/slim/efe",
    "minecraft:entity/player/slim/kai",
    "minecraft:entity/player/slim/makena",
    "minecraft:entity/player/slim/noor",
    "minecraft:entity/player/slim/steve",
    "minecraft:entity/player/slim/sunny",
    "minecraft:entity/player/slim/zuri",
    "minecraft:entity/player/wide/alex",
    "minecraft:entity/player/wide/ari",
    "minecraft:entity/player/wide/efe",
    "minecraft:entity/player/wide/kai",
    "minecraft:entity/player/wide/makena",
    "minecraft:entity/player/wide/noor",
    "minecraft:entity/player/wide/steve",
    "minecraft:entity/player/wide/sunny",
    "minecraft:entity/player/wide/zuri",
    "minecraft:entity/steve",
    "minecraft:entity/alex",
    "minecraft:entity/zombie_pigman",
    "minecraft:entity/shulker/shulker_silver",
    "minecraft:entity/villager/farmer",
    "minecraft:entity/villager/librarian",
    "minecraft:entity/villager/priest",
    "minecraft:entity/villager/smith",
    "minecraft:entity/villager/butcher",
];
pub(crate) fn has_invisible_layer(id: &str) -> bool {
    eyes::visible_when_invisible(id)
        || objects::editor_location(id)
        || matches!(id, "minecraft:strider" | "minecraft:wither")
}
pub(crate) fn villager_texture_dependency(id: &str) -> bool {
    villagers::texture_dependency(id)
}
#[derive(Clone, Debug)]
struct ManagedBlockVisual {
    state: mcedit2_world::sections::State,
    chunk: (i32, i32),
    group: Option<usize>,
}
fn managed_block_source(marker: &Marker) -> Result<([i32; 3], ManagedBlockVisual), String> {
    if marker.position.iter().any(|value| {
        !value.is_finite()
            || value.fract() != 0.0
            || *value < i32::MIN as f64
            || *value > i32::MAX as f64
    }) {
        return Err("方块实体托管位置不是可表示的整数方块坐标".into());
    }
    let position = marker.position.map(|value| value as i32);
    let state = marker
        .block_state
        .as_deref()
        .ok_or("方块实体托管几何缺少源方块状态")?
        .clone();
    Ok((
        position,
        ManagedBlockVisual {
            state,
            chunk: (position[0].div_euclid(16), position[2].div_euclid(16)),
            group: None,
        },
    ))
}
#[derive(Debug)]
pub(crate) struct Frame {
    pub snapshot: Arc<Snapshot>,
    pub show_textures: bool,
    pub show_entities: bool,
    pub visibility: Visibility,
    pub atlas: Arc<TextureAtlas>,
    pub chunks: crate::picking::Chunks,
    entities: BTreeMap<(i32, i32), Vec<MarkerRef>>,
    /// 只有成功解释的真实方块实体图层才替换二维格网的整格面。
    managed_blocks: BTreeMap<[i32; 3], ManagedBlockVisual>,
    resource_identity: crate::textures::VisibilityIdentity,
    pub rendered: usize,
    pub missing: usize,
    pub invisible: usize,
    pub diagnostics: Vec<String>,
    pub player_names: BTreeMap<String, String>,
    assembly: Option<Arc<scene::Assembly>>,
}
impl Frame {
    pub(crate) fn displays(&self, enabled: bool, visibility: Visibility) -> bool {
        self.show_entities == enabled && self.visibility == visibility
    }
    /// 当前源状态、资源身份和用于联合光栅的完整图集必须同时匹配。
    /// 旧帧、异源格网或尚未完成的投影不能仅凭一个位置标记抑制地形。
    pub(crate) fn manages_block_visual(
        &self,
        position: [i32; 3],
        state: &mcedit2_world::sections::State,
        bank: &TextureBank,
        atlas: &TextureAtlas,
    ) -> bool {
        self.managed_blocks
            .get(&position)
            .is_some_and(|entry| &entry.state == state)
            && self.resource_identity == bank.visibility_id()
            && std::ptr::eq(self.atlas.as_ref(), atlas)
    }
    /// 借用已提交的原始四角，不复制几何；供切片复用生产裁剪检查是否确有可见面。
    pub(crate) fn block_visual_faces(
        &self,
        position: [i32; 3],
        state: &mcedit2_world::sections::State,
        bank: &TextureBank,
        atlas: &TextureAtlas,
    ) -> Option<([i32; 3], &[WorldQuad])> {
        if !self.manages_block_visual(position, state, bank, atlas) {
            return None;
        }
        let entry = self.managed_blocks.get(&position)?;
        let Some(group) = entry.group else {
            return Some(([entry.chunk.0 * 16, 0, entry.chunk.1 * 16], &[]));
        };
        let index = self
            .chunks
            .binary_search_by_key(&entry.chunk, |chunk| (chunk.key.0, chunk.key.1))
            .ok()?;
        let chunk = &self.chunks[index];
        let range = &chunk.picking.source_groups().get(group)?.quads;
        Some((chunk.origin, chunk.quads().get(range.clone())?))
    }
    pub fn marker(&self, chunk: (i32, i32), group: usize) -> Option<&MarkerRef> {
        self.entities.get(&chunk)?.get(group)
    }
    #[cfg(test)]
    pub(crate) fn build(
        snapshot: Arc<Snapshot>,
        bank: &TextureBank,
        version: Option<i32>,
        show_textures: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        Self::build_with_root(
            snapshot,
            bank,
            version,
            show_textures,
            true,
            cancelled,
            None,
            false,
            &SkinResults::new(),
            &HeadResults::new(),
            crate::language::Language::English,
        )
    }
    #[cfg(test)]
    fn build_with_root(
        snapshot: Arc<Snapshot>,
        bank: &TextureBank,
        version: Option<i32>,
        show_textures: bool,
        show_entities: bool,
        cancelled: &dyn Fn() -> bool,
        root: Option<&std::path::Path>,
        online_skins: bool,
        skin_results: &SkinResults,
        head_results: &HeadResults,
        language: crate::language::Language,
    ) -> Result<Self, String> {
        scene::assemble(
            snapshot,
            bank,
            version,
            show_textures,
            show_entities,
            cancelled,
            root,
            online_skins,
            skin_results,
            head_results,
            language,
            None,
            None,
            None,
            Visibility::default(),
            None,
        )
    }
    fn prepare(
        markers: &[MarkerRef],
        bank: &TextureBank,
        version: Option<i32>,
        show_textures: bool,
        show_entities: bool,
        cancelled: &dyn Fn() -> bool,
        root: Option<&std::path::Path>,
        online_skins: bool,
        skin_results: &SkinResults,
        head_results: &HeadResults,
        language: crate::language::Language,
        fonts: &mut crate::font_glyphs::Fonts<'_>,
        item_world: Option<&crate::item_world::Environment>,
        maps: Option<&mcedit2_world::map_edit::View>,
    ) -> Result<scene::Prepared, String> {
        let base_atlas = bank
            .atlas()
            .cloned()
            .ok_or("实体模型缺少已完成的纹理图集")?;
        let mut runtime_textures = BTreeMap::new();
        let mut frames = BTreeMap::new();
        let mut map_diagnostics = Vec::new();
        if show_entities {
            for (index, entity) in markers.iter().enumerate() {
                if cancelled() {
                    return Err("物品实体外观准备已取消".into());
                }
                let marker = entity.marker();
                if !marker
                    .id
                    .as_deref()
                    .is_some_and(crate::item_geometry::supported)
                    || marker
                        .id
                        .as_deref()
                        .is_some_and(|id| bank.has_declarative_entity(id))
                {
                    continue;
                }
                let prepared = marker.visual_error.as_ref().map_or_else(
                    || {
                        marker
                            .visual
                            .as_deref()
                            .ok_or_else(|| "物品实体缺少源外观".to_string())
                            .and_then(|visual| {
                                crate::item_geometry::prepare(
                                    visual,
                                    marker.id.as_deref().ok_or("物品实体缺少 ID")?,
                                    marker.position,
                                    bank,
                                    root,
                                    version,
                                    online_skins,
                                    cancelled,
                                    item_world.map(|environment| {
                                        crate::frame_geometry::ItemContext {
                                            environment,
                                            offset: [0.; 3],
                                        }
                                    }),
                                    language,
                                    maps,
                                )
                            })
                    },
                    |error| Err(error.clone()),
                );
                if let Ok(prepared) = &prepared {
                    for (id, texture) in &prepared.textures {
                        runtime_textures
                            .entry(id.clone())
                            .or_insert_with(|| Arc::clone(texture));
                    }
                    map_diagnostics.extend(
                        prepared
                            .diagnostics
                            .iter()
                            .map(|d| format!("物品实体 {}：{d}", marker.path)),
                    );
                }
                frames.insert(index, prepared);
            }
        }
        let mut skins = BTreeMap::<usize, Result<Option<(String, bool)>, String>>::new();
        let mut player_names = BTreeMap::new();
        if show_entities && online_skins {
            for (index, entity) in markers
                .iter()
                .enumerate()
                .filter(|(_, e)| e.marker().kind == Kind::Player)
            {
                if cancelled() {
                    return Err("玩家皮肤加载已取消".into());
                }
                let result = player_uuid(entity.marker())
                    .ok_or_else(|| "玩家 UUID 缺失".to_string())
                    .and_then(|uuid| {
                        skin_results
                            .get(&uuid)
                            .cloned()
                            .unwrap_or_else(|| Err("在线皮肤正在后台加载".into()))
                    })
                    .map(|skin| {
                        let result = skin
                            .custom()
                            .map(|skin| (skin.texture.id.clone(), skin.slim));
                        if let Some(name) = skin.name() {
                            player_names.insert(entity.marker().sha256.clone(), name.to_owned());
                        }
                        map_diagnostics.push(format!(
                            "玩家 {} 皮肤：{}",
                            skin.name().unwrap_or("未提供名称"),
                            if skin.cached() {
                                "本地缓存"
                            } else {
                                "Mojang 公共服务"
                            }
                        ));
                        if let Some(custom) = skin.custom() {
                            runtime_textures
                                .entry(custom.texture.id.clone())
                                .or_insert_with(|| Arc::new(custom.texture.clone()));
                        }
                        result
                    });
                if let Err(error) = &result {
                    map_diagnostics.push(format!("玩家皮肤：{error}"));
                }
                skins.insert(index, result);
            }
        }
        let mut equipment = BTreeMap::new();
        if show_entities {
            for (index, source) in markers.iter().enumerate() {
                if cancelled() {
                    return Err("实体手持物准备已取消".into());
                }
                let marker = source.marker();
                if !matches!(marker.kind, Kind::Entity | Kind::Player)
                    || marker
                        .id
                        .as_deref()
                        .is_some_and(|id| bank.has_declarative_entity(id))
                {
                    continue;
                }
                let Some(visual) = marker.visual.as_deref() else {
                    continue;
                };
                let skin = if marker.kind == Kind::Player && online_skins {
                    let uuid = player_uuid(marker).ok_or("装备持有者缺少玩家 UUID")?;
                    match skin_results.get(&uuid) {
                        Some(Ok(skin)) => Some(skin.as_ref()),
                        _ => continue,
                    }
                } else {
                    None
                };
                match crate::equipment_geometry::prepare(
                    visual,
                    marker.kind,
                    marker.id.as_deref(),
                    marker.position,
                    skin,
                    bank,
                    online_skins,
                    cancelled,
                    item_world.map(|environment| crate::item_geometry::ItemContext {
                        environment,
                        offset: [0.; 3],
                    }),
                ) {
                    Ok(prepared) => {
                        for (id, texture) in &prepared.textures {
                            runtime_textures
                                .entry(id.clone())
                                .or_insert_with(|| Arc::clone(texture));
                        }
                        map_diagnostics.extend(
                            prepared
                                .diagnostics
                                .iter()
                                .map(|error| format!("实体 {}：{error}", marker.path)),
                        );
                        if !prepared.faces.is_empty() {
                            equipment.insert(index, prepared);
                        }
                    }
                    Err(error) => {
                        if cancelled() {
                            return Err("实体手持物准备已取消".into());
                        }
                        let error = format!("实体装备 {}：{error}", marker.path);
                        log::error!("[equipment] {error}");
                        map_diagnostics.push(error);
                    }
                }
            }
        }
        let mut names = BTreeMap::new();
        if show_entities {
            for (index, source) in markers.iter().enumerate() {
                if cancelled() {
                    return Err("实体名称准备已取消".into());
                }
                let marker = source.marker();
                if !matches!(marker.kind, Kind::Entity | Kind::Item | Kind::Player) {
                    continue;
                }
                let Some(visual) = marker.visual.as_deref() else {
                    continue;
                };
                let profile = if marker.kind == Kind::Player {
                    player_names.get(&marker.sha256).map(String::as_str)
                } else {
                    None
                };
                match crate::name_geometry::prepare(
                    visual, profile, bank, fonts, language, cancelled,
                ) {
                    Ok(Some(name)) => {
                        for texture in name.textures() {
                            runtime_textures
                                .entry(texture.id.clone())
                                .or_insert_with(|| Arc::clone(texture));
                        }
                        names.insert(index, name);
                    }
                    Ok(None) => (),
                    Err(error) => {
                        if cancelled() {
                            return Err("实体名称准备已取消".into());
                        }
                        let error = format!("实体名称 {}：{error}", marker.path);
                        log::error!("[entity-name] {error}");
                        map_diagnostics.push(error);
                    }
                }
            }
        }
        let mut block_visuals = BTreeMap::new();
        let mut sign_materials = BTreeMap::new();
        let mut banner_materials = BTreeMap::new();
        for (index, entity) in markers.iter().enumerate() {
            if cancelled() {
                return Err("方块实体外观准备已取消".into());
            }
            let marker = entity.marker();
            let Some(state) = marker.block_state.as_ref() else {
                continue;
            };
            if bank.declarative_block_replaces(state) == Some(true) {
                continue;
            }
            if !bank.vanilla_banner_supported(state)
                && !bank.vanilla_skull_supported(state)
                && !bank.vanilla_sign_supported(state)
                && !bank.vanilla_piston_supported(state)
            {
                continue;
            }
            let prepared = marker
                .block_visual
                .as_ref()
                .ok_or_else(|| {
                    marker
                        .visual_error
                        .clone()
                        .unwrap_or_else(|| "方块缺少可解析的源方块实体".into())
                })
                .and_then(|visual| match visual.as_ref() {
                    BlockVisual::Piston(source) => Ok((BlockVisual::Piston(source.clone()), None)),
                    BlockVisual::Sign(source) => {
                        let prepared = if show_textures {
                            crate::sign_geometry::prepare(source, bank, fonts, language, cancelled)?
                        } else {
                            Arc::new(crate::sign_geometry::Prepared {
                                sides: [None, None],
                            })
                        };
                        for texture in prepared.textures() {
                            if !base_atlas.entries.contains_key(&texture.id) {
                                runtime_textures
                                    .entry(texture.id.clone())
                                    .or_insert_with(|| Arc::clone(texture));
                            }
                        }
                        sign_materials.insert(index, prepared);
                        Ok((BlockVisual::Sign(source.clone()), None))
                    }
                    BlockVisual::Banner(source) => {
                        let material = crate::banner_geometry::prepare(source, bank, cancelled)?;
                        for texture in material.textures() {
                            if !base_atlas.entries.contains_key(&texture.id) {
                                runtime_textures
                                    .entry(texture.id.clone())
                                    .or_insert_with(|| Arc::clone(texture));
                            }
                        }
                        banner_materials.insert(index, material);
                        Ok((BlockVisual::Banner(source.clone()), None))
                    }
                    BlockVisual::Skull(source) => {
                        let mut source = source.clone();
                        let mut texture_id = None;
                        if let Some(profile) = source
                            .profile
                            .as_ref()
                            .filter(|_| source.kind == mcedit2_world::skull::Kind::Player)
                        {
                            let key =
                                crate::skull_skin::key(profile, source.legacy_rotation.is_some());
                            if let Some(result) = head_results.get(&key) {
                                let appearance = result.as_ref().map_err(Clone::clone)?;
                                map_diagnostics.push(format!(
                                    "头颅 {}：{}",
                                    marker.path, appearance.diagnostic
                                ));
                                source.profile = Some(appearance.profile.clone());
                                if let Some(skin) = &appearance.skin {
                                    let id = skin.texture.id.clone();
                                    if !base_atlas.entries.contains_key(&id) {
                                        runtime_textures
                                            .entry(id.clone())
                                            .or_insert_with(|| Arc::new(skin.texture.clone()));
                                    }
                                    texture_id = Some(id);
                                }
                            } else {
                                map_diagnostics.push(format!(
                                    "头颅 {}：皮肤正在后台准备，当前使用原版默认外观",
                                    marker.path
                                ));
                            }
                        }
                        Ok((BlockVisual::Skull(source), texture_id))
                    }
                });
            if cancelled() {
                return Err("方块实体外观准备已取消".into());
            }
            block_visuals.insert(index, prepared);
        }
        Ok(scene::Prepared {
            language,
            equipment,
            names,
            runtime_textures,
            frames,
            skins,
            block_visuals,
            sign_materials,
            banner_materials,
            player_names,
            diagnostics: map_diagnostics,
        })
    }
    fn compile_prepared(
        snapshot: Arc<Snapshot>,
        bank: &TextureBank,
        show_textures: bool,
        show_entities: bool,
        visibility: Visibility,
        cancelled: &dyn Fn() -> bool,
        online_skins: bool,
        prepared: &scene::Prepared,
        atlas: Arc<TextureAtlas>,
        selected: Option<&MarkerRef>,
    ) -> Result<(Self, BTreeMap<String, usize>), String> {
        let render_bank = bank;
        let frames = &prepared.frames;
        let skins = &prepared.skins;
        let block_visuals = &prepared.block_visuals;
        let sign_materials = &prepared.sign_materials;
        let banner_materials = &prepared.banner_materials;
        let mut frame = Self {
            snapshot,
            show_textures,
            show_entities,
            visibility,
            atlas,
            chunks: Default::default(),
            entities: BTreeMap::new(),
            managed_blocks: BTreeMap::new(),
            resource_identity: bank.visibility_id(),
            rendered: 0,
            missing: 0,
            invisible: 0,
            diagnostics: Vec::new(),
            player_names: prepared.player_names.clone(),
            assembly: None,
        };
        let mut groups =
            BTreeMap::<(i32, i32), (Vec<WorldQuad>, Vec<crate::picking::Group>)>::new();
        let mut diagnostics = BTreeMap::<String, usize>::new();
        for (index, entity) in frame.snapshot.markers.iter().enumerate() {
            if cancelled() {
                return Err("实体模型编译已取消".into());
            }
            let marker = entity.marker();
            if !visibility.allows(marker) {
                if marker
                    .block_state
                    .as_deref()
                    .is_some_and(|state| render_bank.block_entity_replaces_base(state))
                {
                    let (position, source) = managed_block_source(marker)?;
                    frame.managed_blocks.insert(position, source);
                }
                frame.invisible += 1;
                continue;
            }
            if let Err(error) = render_placement(marker.center()) {
                frame.missing += 1;
                *diagnostics
                    .entry(format!("实体 {}：{error}；保留来源位置标记", marker.path))
                    .or_default() += 1;
                continue;
            }
            let native = block_visuals.get(&index).map(|prepared| {
                prepared
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|(visual, id)| match visual {
                        BlockVisual::Piston(source) => crate::piston_geometry::quads(
                            source,
                            render_bank,
                            marker.position.map(|v| v as i32),
                            &frame.snapshot.biomes,
                            show_textures,
                            cancelled,
                        ),
                        BlockVisual::Sign(source) => crate::sign_geometry::quads(
                            source,
                            sign_materials.get(&index).ok_or("告示牌缺少已准备文字")?,
                            &render_bank,
                            &frame.atlas,
                            show_textures,
                        ),
                        BlockVisual::Banner(source) => crate::banner_geometry::quads(
                            source,
                            &render_bank,
                            &frame.atlas,
                            banner_materials.get(&index).ok_or("旗帜缺少已准备材质")?,
                            show_textures,
                        ),
                        BlockVisual::Skull(source) => crate::skull_geometry::quads(
                            source,
                            &render_bank,
                            &frame.atlas,
                            id.as_deref(),
                            show_textures,
                        ),
                    })
            });
            let declared = marker.block_state.as_deref().and_then(|state| {
                render_bank.declarative_block_quads(state, marker.visual.as_deref(), show_textures)
            });
            let visual = match (declared, native) {
                (Some((true, result)), _) => Some((true, result)),
                (Some((false, extra)), Some(native)) => Some((
                    true,
                    native.and_then(|mut faces| {
                        faces.extend(extra?);
                        Ok(faces)
                    }),
                )),
                (Some((false, result)), None) => Some((false, result)),
                (None, Some(result)) => Some((true, result)),
                (None, None) => None,
            };
            if let Some((replaces, result)) = visual {
                let quads = match result {
                    Ok(quads) => {
                        if quads.is_empty() {
                            frame.invisible += 1;
                        } else {
                            frame.rendered += 1;
                        }
                        quads
                    }
                    Err(error) => {
                        frame.missing += 1;
                        *diagnostics
                            .entry(format!(
                                "方块实体 {}：{error}；保留来源位置标记",
                                marker.path
                            ))
                            .or_default() += 1;
                        continue;
                    }
                };
                let has_faces = !quads.is_empty();
                let (position, mut managed) = managed_block_source(marker)?;
                append_geometry(&mut groups, &mut frame.entities, entity, quads)?;
                let chunk = managed.chunk;
                let group = if has_faces {
                    Some(
                        frame
                            .entities
                            .get(&chunk)
                            .and_then(|entries| entries.len().checked_sub(1))
                            .ok_or("方块实体托管几何缺少实际源组")?,
                    )
                } else {
                    None
                };
                if replaces {
                    managed.group = group;
                    frame.managed_blocks.insert(position, managed);
                }
                continue;
            }
            if !show_entities {
                continue;
            }
            if marker
                .id
                .as_deref()
                .is_some_and(mcedit2_world::painting::is_painting)
            {
                let declared = marker.visual.as_deref().and_then(|visual| {
                    marker
                        .id
                        .as_deref()
                        .and_then(|id| bank.declarative_entity_quads(id, visual))
                });
                let result = if let Some(declared) = declared {
                    declared.map(|faces| (marker.center(), faces))
                } else {
                    marker.visual_error.as_ref().map_or_else(
                        || {
                            marker
                                .visual
                                .as_deref()
                                .ok_or_else(|| "挂画缺少源外观".to_string())
                                .and_then(|visual| {
                                    crate::painting_geometry::quads(
                                        visual,
                                        bank,
                                        &frame.atlas,
                                        show_textures,
                                    )
                                })
                        },
                        |error| Err(error.clone()),
                    )
                };
                let (center, quads) = match result {
                    Ok((center, quads)) => {
                        frame.rendered += 1;
                        let mut quads = with_name(
                            index,
                            entity,
                            center,
                            quads,
                            prepared,
                            selected,
                            &frame.atlas,
                            &mut diagnostics,
                        );
                        append_fire(
                            entity,
                            center,
                            &mut quads,
                            bank,
                            &frame.atlas,
                            show_textures,
                            &mut diagnostics,
                        );
                        (center, quads)
                    }
                    Err(error) => {
                        frame.missing += 1;
                        *diagnostics
                            .entry(format!("挂画 {}：{error}；保留来源位置标记", marker.path))
                            .or_default() += 1;
                        continue;
                    }
                };
                append_geometry_at(&mut groups, &mut frame.entities, entity, center, quads)?;
                continue;
            }
            if marker
                .id
                .as_deref()
                .is_some_and(crate::item_geometry::supported)
            {
                let declared = marker.id.as_deref().and_then(|id| {
                    marker
                        .visual
                        .as_deref()
                        .and_then(|visual| bank.declarative_entity_quads(id, visual))
                });
                let result = match declared {
                    Some(result) => result.map(|quads| (marker.center(), quads)),
                    None => frames
                        .get(&index)
                        .ok_or("物品实体源外观未准备")?
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|p| Ok((p.center, p.quads(&frame.atlas, show_textures)?))),
                };
                let (center, quads) = match result {
                    Ok((center, quads)) => {
                        let mut quads = with_name(
                            index,
                            entity,
                            center,
                            quads,
                            prepared,
                            selected,
                            &frame.atlas,
                            &mut diagnostics,
                        );

                        append_fire(
                            entity,
                            center,
                            &mut quads,
                            bank,
                            &frame.atlas,
                            show_textures,
                            &mut diagnostics,
                        );
                        if quads.is_empty() {
                            frame.invisible += 1;
                        } else {
                            frame.rendered += 1;
                        }
                        (center, quads)
                    }
                    Err(error) => {
                        frame.missing += 1;
                        *diagnostics
                            .entry(format!(
                                "物品实体 {}：{error}；保留来源位置标记",
                                marker.path
                            ))
                            .or_default() += 1;
                        continue;
                    }
                };
                append_geometry_at(&mut groups, &mut frame.entities, entity, center, quads)?;
                continue;
            }
            if marker
                .id
                .as_deref()
                .is_some_and(mcedit2_world::entity_catalog::is_experience_orb)
            {
                let declared = marker.id.as_deref().and_then(|id| {
                    marker
                        .visual
                        .as_deref()
                        .and_then(|visual| bank.declarative_entity_quads(id, visual))
                });
                let result = match declared {
                    Some(result) => result,
                    None => marker.visual_error.as_ref().map_or_else(
                        || {
                            marker
                                .visual
                                .as_deref()
                                .ok_or_else(|| "经验球缺少源外观".to_string())
                                .and_then(|visual| {
                                    crate::experience_geometry::quads(
                                        visual,
                                        bank,
                                        &frame.atlas,
                                        show_textures,
                                    )
                                })
                        },
                        |error| Err(error.clone()),
                    ),
                };
                let quads = match result {
                    Ok(quads) => {
                        if quads.is_empty() {
                            frame.invisible += 1;
                        } else {
                            frame.rendered += 1;
                        }
                        let mut quads = with_name(
                            index,
                            entity,
                            marker.center(),
                            quads,
                            prepared,
                            selected,
                            &frame.atlas,
                            &mut diagnostics,
                        );
                        append_fire(
                            entity,
                            marker.center(),
                            &mut quads,
                            bank,
                            &frame.atlas,
                            show_textures,
                            &mut diagnostics,
                        );
                        quads
                    }
                    Err(error) => {
                        frame.missing += 1;
                        *diagnostics
                            .entry(format!("经验球 {}：{error}；保留来源位置标记", marker.path))
                            .or_default() += 1;
                        continue;
                    }
                };
                append_geometry_at(
                    &mut groups,
                    &mut frame.entities,
                    entity,
                    marker.center(),
                    quads,
                )?;
                continue;
            }
            if !matches!(marker.kind, Kind::Entity | Kind::Player) {
                continue;
            }
            let id = if marker.kind == Kind::Player {
                "玩家"
            } else {
                marker.id.as_deref().unwrap_or("<缺少实体 ID>")
            };
            let invisible = marker.visual.as_ref().is_some_and(
                |v| matches!(v.appearance.get("Invisible"), Some(Value::Byte(n)) if *n != 0),
            );
            let has_name = prepared.names.get(&index).is_some_and(|name| {
                name.always_visible || selected.is_some_and(|s| s.same(entity))
            });
            let has_equipment = prepared.equipment.contains_key(&index);
            let has_fire = match marker
                .visual
                .as_deref()
                .map(crate::entity_fire::present)
                .transpose()
            {
                Ok(value) => value == Some(true),
                Err(error) => {
                    *diagnostics
                        .entry(format!("实体火焰 {}：{error}", marker.path))
                        .or_default() += 1;
                    false
                }
            };
            if id == "minecraft:marker"
                || invisible
                    && !has_name
                    && !has_equipment
                    && !has_fire
                    && !has_invisible_layer(id)
                    && !(marker.kind == Kind::Player
                        && marker
                            .visual
                            .as_ref()
                            .is_some_and(|visual| parrots::has_shoulders(visual)))
            {
                frame.invisible += 1;
                continue;
            }
            let declared = if marker.kind == Kind::Player {
                None
            } else {
                marker.id.as_deref().and_then(|id| {
                    marker
                        .visual
                        .as_deref()
                        .and_then(|visual| render_bank.declarative_entity_quads(id, visual))
                })
            };
            let result = if let Some(result) = declared {
                result
            } else {
                if marker.kind == Kind::Player && online_skins {
                    match skins.get(&index) {
                        Some(Ok(binding)) => compile(
                            &render_bank,
                            entity,
                            binding.as_ref().map(|(id, slim)| (id.as_str(), *slim)),
                            prepared.language,
                        ),
                        Some(Err(error)) => Err(error.clone()),
                        None => Err("玩家在线皮肤结果缺失".into()),
                    }
                } else {
                    compile(&render_bank, entity, None, prepared.language)
                }
            };
            let result = result.and_then(|quads| {
                if let Some(error) = &marker.visual_error {
                    return Err(error.clone());
                }
                Ok(quads)
            });
            let mut quads = match result {
                Ok(quads) => {
                    if invisible {
                        frame.invisible += 1;
                    } else {
                        frame.rendered += 1;
                    }
                    quads
                }
                Err(error) => {
                    *diagnostics
                        .entry(format!("{id}：{error}；保留来源位置标记"))
                        .or_default() += 1;
                    frame.missing += 1;
                    continue;
                }
            };
            if !show_textures {
                quads.retain(|quad| quad.mode & crate::material_mode::ADDITIVE == 0);
                for quad in &mut quads {
                    for color in &mut quad.rgba {
                        if crate::material_mode::pre_tint_alpha(color[3]) {
                            color[3] = 1.;
                        }
                    }
                    quad.uv = [frame.atlas.white.sample(0.5, 0.5); 4];
                    quad.mode = crate::material_mode::untextured(quad.mode);
                    quad.atlas_page = frame.atlas.white.page;
                }
            }
            {
                let body = quads.len();
                quads = with_name(
                    index,
                    entity,
                    marker.center(),
                    quads,
                    prepared,
                    selected,
                    &frame.atlas,
                    &mut diagnostics,
                );
                if invisible {
                    let mut index = 0;
                    quads.retain(|quad| {
                        let keep = index >= body
                            || quad.mode & crate::material_mode::VISIBLE_WHEN_INVISIBLE != 0;
                        index += 1;
                        keep
                    });
                }
                if let Some(hands) = prepared.equipment.get(&index) {
                    quads.extend(hands.quads(&frame.atlas, show_textures)?);
                }
                append_fire(
                    entity,
                    marker.center(),
                    &mut quads,
                    bank,
                    &frame.atlas,
                    show_textures,
                    &mut diagnostics,
                );
            }
            append_geometry(&mut groups, &mut frame.entities, entity, quads)?;
        }
        let mut chunks = Vec::with_capacity(groups.len());
        for (coordinate, (mut faces, owners)) in groups {
            if cancelled() {
                return Err("实体模型索引已取消".into());
            }
            faces.shrink_to_fit();
            let origin = [coordinate.0 * 16, 0, coordinate.1 * 16];
            let quads = Arc::new(faces);
            let picking = Arc::new(crate::picking::Index::build(
                origin,
                Arc::clone(&quads),
                owners,
            )?);
            chunks.push(MeshChunk::new(
                (coordinate.0, coordinate.1, 0, 0, 0),
                origin,
                quads,
                picking,
            ));
        }
        frame.chunks = chunks.into();
        frame
            .diagnostics
            .extend(prepared.diagnostics.iter().cloned());
        Ok((frame, diagnostics))
    }
}
fn append_fire(
    entity: &MarkerRef,
    center: [f64; 3],
    quads: &mut Vec<WorldQuad>,
    bank: &TextureBank,
    atlas: &TextureAtlas,
    textures: bool,
    diagnostics: &mut BTreeMap<String, usize>,
) {
    let marker = entity.marker();
    let result = (|| {
        let id = if marker.kind == Kind::Player {
            "minecraft:player"
        } else {
            marker.id.as_deref().ok_or("燃烧实体缺少实际 ID")?
        };
        crate::entity_fire::quads(
            id,
            marker.visual.as_deref().ok_or("燃烧实体缺少实际外观")?,
            marker.position,
            center,
            bank,
            atlas,
            textures,
            &entity.chunk.lighting,
            &BTreeMap::new(),
        )
    })();
    match result {
        Ok(faces) => quads.extend(faces),
        Err(error) => {
            let error = format!("实体火焰 {}：{error}", marker.path);
            log::error!("[entity-fire] {error}");
            *diagnostics.entry(error).or_default() += 1;
        }
    }
}
fn name_quads(
    index: usize,
    entity: &MarkerRef,
    center: [f64; 3],
    model: &[WorldQuad],
    prepared: &scene::Prepared,
    selected: Option<&MarkerRef>,
    atlas: &TextureAtlas,
) -> Result<Vec<WorldQuad>, String> {
    let Some(name) = prepared.names.get(&index) else {
        return Ok(Vec::new());
    };
    if !name.always_visible && !selected.is_some_and(|s| s.same(entity)) {
        return Ok(Vec::new());
    }
    let marker = entity.marker();
    let top = if let Some(geometry) = &marker.geometry {
        geometry.bounds().1[1] - center[1]
    } else {
        f64::from(
            model
                .iter()
                .map(|q| q.bounds().1[1])
                .reduce(f32::max)
                .ok_or("实际模型未提供名称标签范围")?,
        )
    };
    let anchor = [
        (marker.center()[0] - center[0]) as f32,
        (top + 0.5) as f32,
        (marker.center()[2] - center[2]) as f32,
    ];
    name.quads(atlas, anchor)
}
fn with_name(
    index: usize,
    entity: &MarkerRef,
    center: [f64; 3],
    mut model: Vec<WorldQuad>,
    prepared: &scene::Prepared,
    selected: Option<&MarkerRef>,
    atlas: &TextureAtlas,
    diagnostics: &mut BTreeMap<String, usize>,
) -> Vec<WorldQuad> {
    match name_quads(index, entity, center, &model, prepared, selected, atlas) {
        Ok(names) => model.extend(names),
        Err(error) => {
            let error = format!("实体名称 {}：{error}", entity.marker().path);
            log::error!("[entity-name] {error}");
            *diagnostics.entry(error).or_default() += 1;
        }
    }
    model
}

type Groups = BTreeMap<(i32, i32), (Vec<WorldQuad>, Vec<crate::picking::Group>)>;
fn append_geometry(
    groups: &mut Groups,
    entities: &mut BTreeMap<(i32, i32), Vec<MarkerRef>>,
    entity: &MarkerRef,
    quads: Vec<WorldQuad>,
) -> Result<(), String> {
    let center = entity.marker().center();
    append_geometry_at(groups, entities, entity, center, quads)
}

/// 原点与局部浮点位置属于几何合法性，不依赖已移除的光照探针校验。
fn render_placement(center: [f64; 3]) -> Result<((i32, i32), [f32; 3]), String> {
    let axis = |value: f64| -> Result<(i32, i32), String> {
        let origin = (value / 16.0).floor() * 16.0;
        if !origin.is_finite() || origin < i32::MIN as f64 || origin > i32::MAX as f64 {
            return Err(format!("实体位置 {value} 超出可表示的渲染原点"));
        }
        let origin = origin as i32;
        Ok((origin / 16, origin))
    };
    let (x, ox) = axis(center[0])?;
    let (z, oz) = axis(center[2])?;
    let coordinate = (x, z);
    let origin = [ox, 0, oz];
    let delta = std::array::from_fn(|i| (center[i] - origin[i] as f64) as f32);
    if !delta.into_iter().all(f32::is_finite) {
        return Err("实体相对渲染区块的位置溢出".into());
    }
    Ok((coordinate, delta))
}

fn append_geometry_at(
    groups: &mut Groups,
    entities: &mut BTreeMap<(i32, i32), Vec<MarkerRef>>,
    entity: &MarkerRef,
    center: [f64; 3],
    quads: Vec<WorldQuad>,
) -> Result<(), String> {
    let (coordinate, delta) = render_placement(center)?;
    let (faces, owners) = groups.entry(coordinate).or_default();
    let start = faces.len();
    faces.extend(quads.into_iter().map(|q| q.translated(delta)));
    if faces.len() > start {
        owners.push(crate::picking::Group {
            quads: start..faces.len(),
            owner: None,
        });
        entities.entry(coordinate).or_default().push(entity.clone());
    }
    Ok(())
}
fn byte(visual: &EntityVisual, key: &str) -> Result<bool, String> {
    match visual.appearance.get(key) {
        None => Ok(false),
        Some(Value::Byte(n)) => Ok(*n != 0),
        _ => Err(format!("{key} 不是 Byte")),
    }
}
/// 原版 SpiderModel.setupAnim 在静止时保留八腿的展开角；构造器本身没有这些姿态。
fn spider_stance(poses: &mut BTreeMap<String, PartPose>) {
    let quarter = std::f32::consts::FRAC_PI_4;
    let eighth = std::f32::consts::FRAC_PI_8;
    for (part, yaw, roll) in [
        ("right_hind_leg", quarter, -quarter),
        ("left_hind_leg", -quarter, quarter),
        ("right_middle_hind_leg", eighth, -0.58119464),
        ("left_middle_hind_leg", -eighth, 0.58119464),
        ("right_middle_front_leg", -eighth, -0.58119464),
        ("left_middle_front_leg", eighth, 0.58119464),
        ("right_front_leg", -quarter, -quarter),
        ("left_front_leg", quarter, quarter),
    ] {
        poses.insert(
            format!("/{part}"),
            PartPose {
                rotation: Some([0., yaw, roll]),
                ..Default::default()
            },
        );
    }
}
fn compile(
    bank: &TextureBank,
    entity: &MarkerRef,
    player_skin: Option<(&str, bool)>,
    language: crate::language::Language,
) -> Result<Vec<WorldQuad>, String> {
    let marker = entity.marker();
    let visual = marker.visual.as_ref().ok_or("源实体的外观 NBT 无效")?;
    let catalog = Catalog::for_data_version(visual.source_version)?;
    compile_visual(
        catalog,
        bank,
        marker.kind,
        marker.id.as_deref(),
        visual,
        player_skin,
        language,
    )
}
pub(crate) fn compile_visual(
    catalog: &Catalog,
    bank: &TextureBank,
    kind: Kind,
    id: Option<&str>,
    visual: &EntityVisual,
    player_skin: Option<(&str, bool)>,
    language: crate::language::Language,
) -> Result<Vec<WorldQuad>, String> {
    if kind != Kind::Player {
        if let Some(result) = id.and_then(|id| bank.declarative_entity_quads(id, visual)) {
            return result;
        }
    }
    let [yaw, pitch] = visual.rotation.ok_or("源实体缺少 Rotation")?;
    byte(visual, "Invisible")?;
    if kind == Kind::Player {
        let Some(Value::IntArray(words)) = visual.appearance.get("mcedit2:player_uuid") else {
            return Err("玩家 UUID 缺失，无法确定官方默认皮肤".into());
        };
        if words.len() != 4 {
            return Err("玩家 UUID 无效".into());
        }
        let hash = words[0] ^ words[1] ^ words[2] ^ words[3];
        let index = hash.rem_euclid(18) as usize;
        let legacy = catalog.version == "1.12.2";
        let names = [
            "alex", "ari", "efe", "kai", "makena", "noor", "steve", "sunny", "zuri",
        ];
        let slim = player_skin.map_or(
            if legacy { hash & 1 != 0 } else { index < 9 },
            |(_, slim)| slim,
        );
        let texture = player_skin
            .map(|(id, _)| id.to_string())
            .unwrap_or_else(|| {
                if legacy {
                    format!("minecraft:entity/{}", if slim { "alex" } else { "steve" })
                } else {
                    format!(
                        "minecraft:entity/player/{}/{}",
                        if slim { "slim" } else { "wide" },
                        names[index % 9]
                    )
                }
            });
        let rig = Rig::read(catalog, kind, id, visual, Some(slim))?;
        let mut quads = npc::render(catalog, bank, rig.layer, &texture, &rig.poses, rig.style)?;
        quads.extend(parrots::shoulders(catalog, bank, visual, &rig)?);
        return Ok(quads);
    }
    let id = id.ok_or("源实体缺少 ID")?;
    if id == "minecraft:block_display" {
        return crate::item_geometry::displays::block_quads(visual, bank);
    }
    if matches!(
        id,
        "minecraft:mooshroom" | "minecraft:giant" | "minecraft:ender_dragon"
    ) {
        return large_mobs::compile(catalog, bank, id, visual);
    }
    if id == "minecraft:enderman" {
        return enderman::compile(catalog, bank, visual);
    }
    if illagers::supports(id) {
        return illagers::compile(catalog, bank, id, visual);
    }
    if id == "minecraft:witch" {
        return witch::compile(catalog, bank, visual);
    }
    if id == "minecraft:vex" {
        return vex::compile(catalog, bank, visual);
    }
    if id == "minecraft:allay" {
        return allay::compile(catalog, bank, visual);
    }
    if monsters::supports(id) {
        return monsters::compile(catalog, bank, id, visual);
    }
    if animals::supports(id) {
        return animals::compile(catalog, bank, id, visual);
    }
    if passives::supports(id) {
        return passives::compile(catalog, bank, id, visual, language);
    }
    if golems::supports(id) {
        return golems::compile(catalog, bank, id, visual);
    }
    if cats::supports(id) {
        return cats::compile(catalog, bank, id, visual);
    }
    if bats_squids::supports(id) {
        return bats_squids::compile(catalog, bank, id, visual);
    }
    if id == "minecraft:parrot" {
        return parrots::compile(catalog, bank, visual);
    }
    if mcedit2_world::entity_water::supports(id) {
        return fish::compile(catalog, bank, id, visual);
    }
    if swimmers::supports(id) {
        return swimmers::compile(catalog, bank, id, visual);
    }
    if bees_phantoms::supports(id) {
        return bees_phantoms::compile(catalog, bank, id, visual);
    }
    if foxes_goats::supports(id) {
        return foxes_goats::compile(catalog, bank, id, visual);
    }
    if pandas_ravagers::supports(id) {
        return pandas_ravagers::compile(catalog, bank, id, visual);
    }
    if nether_beasts::supports(id) {
        return nether_beasts::compile(catalog, bank, id, visual);
    }
    if piglins::supports(id) {
        return piglins::compile(catalog, bank, id, visual);
    }
    if guardians::supports(id) {
        return guardians::compile(catalog, bank, id, visual);
    }
    if id == "minecraft:wither" {
        return wither::compile(catalog, bank, visual);
    }
    if id == "minecraft:warden" {
        return warden::compile(catalog, bank, visual);
    }
    if static_mobs::supports(id) {
        return static_mobs::compile(catalog, bank, id, visual);
    }
    if vehicles::supports(id) {
        return vehicles::compile(catalog, bank, id, visual);
    }
    if objects::supports(id) {
        return objects::compile(catalog, bank, id, visual);
    }
    if matches!(id, "minecraft:zombie_pigman" | "minecraft:shulker") {
        return creatures::compile(catalog, bank, id, visual, yaw, pitch);
    }
    if matches!(
        id,
        "minecraft:sheep"
            | "minecraft:villager"
            | "minecraft:armor_stand"
            | "minecraft:zombie_villager"
            | "minecraft:wandering_trader"
    ) {
        return npc::compile(catalog, bank, id, visual, yaw, pitch);
    }
    if visual
        .appearance
        .get("Age")
        .is_some_and(|v| !matches!(v, Value::Int(_)))
    {
        return Err("Age 不是 Int".into());
    }
    let young = match id {
        "minecraft:cow" | "minecraft:pig" => {
            matches!(visual.appearance.get("Age"),Some(Value::Int(v)) if *v<0)
        }
        _ => false,
    };
    let (layer, texture, head) = match id {
        "minecraft:creeper" => (
            "minecraft:creeper#main",
            "minecraft:entity/creeper/creeper",
            "/head",
        ),
        "minecraft:cow" => ("minecraft:cow#main", "minecraft:entity/cow/cow", "/head"),
        "minecraft:pig" => ("minecraft:pig#main", "minecraft:entity/pig/pig", "/head"),
        "minecraft:frog" => {
            let texture = match visual.appearance.get("variant") {
                Some(Value::String(v)) if v == "minecraft:temperate" => {
                    "minecraft:entity/frog/temperate_frog"
                }
                Some(Value::String(v)) if v == "minecraft:warm" => {
                    "minecraft:entity/frog/warm_frog"
                }
                Some(Value::String(v)) if v == "minecraft:cold" => {
                    "minecraft:entity/frog/cold_frog"
                }
                _ => return Err("青蛙源 variant 缺失或未识别".into()),
            };
            ("minecraft:frog#main", texture, "/root/body/head")
        }
        _ => return Err("尚无内建静态实体模型绑定，专用渲染器留待插件扩展".into()),
    };
    let (poses, style) = {
        let poses = BTreeMap::from([(
            head.into(),
            PartPose {
                rotation: Some([pitch.to_radians(), 0., 0.]),
                ..Default::default()
            },
        )]);
        let groups = if young {
            Some(match id {
                "minecraft:cow" | "minecraft:pig" => {
                    let offsets = if let Some(legacy) = &catalog.legacy {
                        if id == "minecraft:cow" {
                            legacy.young_cow
                        } else {
                            legacy.young_pig
                        }
                    } else if id == "minecraft:cow" {
                        [10., 4.]
                    } else {
                        [4., 4.]
                    };
                    npc::Groups {
                        head_parts: &["/head"],
                        head_scale: 1.,
                        head_translate: [0., offsets[0] / 16., offsets[1] / 16.],
                        body_scale: 0.5,
                        body_translate: [0., 1.5, 0.],
                    }
                }
                _ => unreachable!("已识别的原生幼年模型"),
            })
        } else {
            None
        };
        let style = npc::RenderStyle {
            yaw,
            groups,
            outer: if catalog.version == "1.21.1" {
                npc::scale(visual)?
            } else {
                1.
            },
            tint: [1.; 4],
            ordered: false,
        };
        (poses, style)
    };
    let mut result = npc::render(catalog, bank, layer, texture, &poses, style)?;
    if id == "minecraft:pig" && byte(visual, "Saddle")? {
        result.extend(npc::render(
            catalog,
            bank,
            "minecraft:pig#saddle",
            "minecraft:entity/pig/pig_saddle",
            &poses,
            style,
        )?);
    }
    Ok(result)
}
fn same_selection(a: Option<&MarkerRef>, b: Option<&MarkerRef>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.same(b),
        (None, None) => true,
        _ => false,
    }
}
#[derive(Clone)]
struct Input {
    selected: Option<MarkerRef>,
    item_world: Option<crate::item_world::Environment>,
    language: crate::language::Language,
    snapshot: Arc<Snapshot>,
    bank: Arc<TextureBank>,
    version: Option<i32>,
    show_textures: bool,
    show_entities: bool,
    visibility: Visibility,
    root: std::path::PathBuf,
    maps: Option<mcedit2_world::map_edit::View>,
    online_skins: bool,
    skin_revision: u64,
    skin_results: Arc<SkinResults>,
    head_results: Arc<HeadResults>,
}
impl Input {
    fn same_geometry(&self, other: &Self) -> bool {
        same_selection(self.selected.as_ref(), other.selected.as_ref())
            && self.item_world == other.item_world
            && self.language == other.language
            && Arc::ptr_eq(&self.snapshot, &other.snapshot)
            && Arc::ptr_eq(&self.bank, &other.bank)
            && self.version == other.version
            && self.show_textures == other.show_textures
            && self.show_entities == other.show_entities
            && self.visibility == other.visibility
            && self.root == other.root
            && self.maps == other.maps
            && self.online_skins == other.online_skins
    }
    fn same(&self, other: &Self) -> bool {
        self.same_geometry(other) && self.skin_revision == other.skin_revision
    }
}
pub(crate) struct State {
    pub maps: Option<mcedit2_world::map_edit::View>,
    pub selected: Option<MarkerRef>,
    pub item_world: Option<crate::item_world::Environment>,
    pub language: crate::language::Language,
    pub enabled: bool,
    pub visibility: Visibility,
    pub online_skins: bool,
    pub frame: Option<Arc<Frame>>,
    // 展示失效与可复用的完成结果分开；缓存仅保存一份完成帧，不形成历史链。
    geometry_cache: Option<Arc<Frame>>,
    scene_totals: scene::Statistics,
    pub composite: Arc<Mutex<Option<Arc<StreamMesh>>>>,
    pub status: String,
    request: Option<Input>,
    epoch: u64,
    running: bool,
    cancelled: Arc<AtomicBool>,
    skin_context: Option<(std::path::PathBuf, bool)>,
    skin_epoch: u64,
    skin_requests: u64,
    skin_revision: u64,
    skin_results: crate::appearance_results::WorkingSet<
        [u8; 16],
        Result<Arc<crate::player_skin::Appearance>, String>,
    >,
    skin_queue: VecDeque<[u8; 16]>,
    skin_queued: BTreeSet<[u8; 16]>,
    skin_inflight: Option<(u64, [u8; 16])>,
    skin_snapshot: Option<Arc<Snapshot>>,
    head_context: Option<(std::path::PathBuf, bool)>,
    head_epoch: u64,
    head_requests: u64,
    head_results: crate::appearance_results::WorkingSet<
        crate::skull_skin::Key,
        Result<Arc<crate::skull_skin::Appearance>, String>,
    >,
    head_queue: VecDeque<(crate::skull_skin::Key, mcedit2_world::skull::Profile, bool)>,
    head_inflight: Option<(u64, crate::skull_skin::Key)>,
    head_snapshot: Option<(Arc<Snapshot>, crate::textures::VisibilityIdentity)>,
    head_cancelled: Arc<AtomicBool>,
}
impl State {
    pub(crate) fn set_display(&mut self, enabled: bool, visibility: Visibility) {
        if self.enabled == enabled && self.visibility == visibility {
            return;
        }
        self.enabled = enabled;
        self.visibility = visibility;
        self.frame = None;
        *self.composite.lock().expect("实体合成缓存锁异常") = None;
    }
}
impl Default for State {
    fn default() -> Self {
        Self {
            maps: None,
            selected: None,
            item_world: None,
            language: crate::language::Language::English,
            enabled: true,
            visibility: Visibility::default(),
            online_skins: false,
            frame: None,
            geometry_cache: None,
            scene_totals: scene::Statistics::default(),
            composite: Default::default(),
            status: String::new(),
            request: None,
            epoch: 0,
            running: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            skin_context: None,
            skin_epoch: 0,
            skin_requests: 0,
            skin_revision: 0,
            skin_results: crate::appearance_results::WorkingSet::new(16 << 20),
            skin_queue: VecDeque::new(),
            skin_queued: BTreeSet::new(),
            skin_inflight: None,
            skin_snapshot: None,
            head_context: None,
            head_epoch: 0,
            head_requests: 0,
            head_results: crate::appearance_results::WorkingSet::new(16 << 20),
            head_queue: VecDeque::new(),
            head_inflight: None,
            head_snapshot: None,
            head_cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
}
fn skin_cost(value: &Result<Arc<crate::player_skin::Appearance>, String>) -> usize {
    match value {
        Ok(skin) => skin.bytes(),
        Err(error) => error.len(),
    }
}
fn head_cost(value: &Result<Arc<crate::skull_skin::Appearance>, String>) -> usize {
    match value {
        Ok(appearance) => {
            std::mem::size_of::<crate::skull_skin::Appearance>()
                + appearance.diagnostic.len()
                + appearance.profile.name.as_ref().map_or(0, String::len)
                + appearance
                    .profile
                    .properties
                    .iter()
                    .map(|p| {
                        p.name.len() + p.value.len() + p.signature.as_ref().map_or(0, String::len)
                    })
                    .sum::<usize>()
                + appearance
                    .skin
                    .as_ref()
                    .map_or(0, |skin| skin.texture.rgba.len())
        }
        Err(error) => error.len(),
    }
}
impl State {
    pub(crate) fn appearance_statistics(&self) -> serde_json::Value {
        serde_json::json!({"heads":self.head_results.statistics(),"players":self.skin_results.statistics(),"head_requests":self.head_requests,"player_requests":self.skin_requests,"scene":{"committed":self.scene_totals,"last":self.frame.as_ref().and_then(|frame|frame.assembly.as_ref()).map(|scene|scene.statistics)},
            "head_errors":self.head_results.active().values().filter(|value|value.is_err()).count(),
            "head_textures":self.head_results.active().values().filter(|value|value.as_ref().is_ok_and(|a|a.skin.is_some())).count()})
    }
    pub(crate) fn reload_appearances(&mut self) {
        self.geometry_cache = None;
        self.head_cancelled.store(true, Ordering::Release);
        self.head_cancelled = Arc::new(AtomicBool::new(false));
        self.head_epoch = self.head_epoch.wrapping_add(1);
        self.skin_epoch = self.skin_epoch.wrapping_add(1);
        self.head_results.clear();
        self.skin_results.clear();
        self.head_queue.clear();
        self.skin_queue.clear();
        self.skin_queued.clear();
        self.head_snapshot = None;
        self.skin_snapshot = None;
        self.skin_revision = self.skin_revision.wrapping_add(1);
    }
    pub(crate) fn is_idle(&self) -> bool {
        !self.running
            && self.skin_inflight.is_none()
            && self.skin_queue.is_empty()
            && self.head_inflight.is_none()
            && self.head_queue.is_empty()
    }
    fn maintain_heads(
        &mut self,
        snapshot: Option<&Arc<Snapshot>>,
        bank: Option<&Arc<TextureBank>>,
        root: Option<&std::path::PathBuf>,
        active: bool,
    ) -> Task<Message> {
        // 皮肤像素由 profile 和联网策略决定；资源包及维度只改变当前要显示哪些头颅。
        let context = root.map(|root| (root.clone(), self.online_skins));
        if self.head_context != context {
            self.head_cancelled.store(true, Ordering::Release);
            self.head_cancelled = Arc::new(AtomicBool::new(false));
            self.head_context = context;
            self.head_epoch = self.head_epoch.wrapping_add(1);
            self.skin_revision = self.skin_revision.wrapping_add(1);
            self.head_results.clear();
            self.head_queue.clear();
            self.head_snapshot = None;
        }
        if !active
            && self
                .head_inflight
                .is_some_and(|(epoch, _)| epoch == self.head_epoch)
        {
            self.head_cancelled.store(true, Ordering::Release);
            self.head_cancelled = Arc::new(AtomicBool::new(false));
            self.head_epoch = self.head_epoch.wrapping_add(1);
        }
        let selected = snapshot
            .zip(bank)
            .filter(|_| active && self.head_context.is_some());
        let Some((snapshot, bank)) = selected else {
            // 流式重读暂时没有快照时，profile 仍是不变来源；完成结果进入最近缓存。
            if self.head_snapshot.take().is_some() {
                self.head_queue.clear();
                self.head_results.select(BTreeSet::new(), head_cost);
                self.skin_revision = self.skin_revision.wrapping_add(1);
            }
            return Task::none();
        };
        let bank_identity = bank.visibility_id();
        if self
            .head_snapshot
            .as_ref()
            .is_none_or(|(old, identity)| !Arc::ptr_eq(old, snapshot) || identity != &bank_identity)
        {
            let wanted = snapshot
                .markers
                .iter()
                .filter_map(|entry| {
                    let BlockVisual::Skull(source) = entry.marker().block_visual.as_deref()? else {
                        return None;
                    };
                    if source.kind != mcedit2_world::skull::Kind::Player
                        || !bank.vanilla_skull_supported(&source.state)
                    {
                        return None;
                    }
                    let profile = source.profile.as_ref()?;
                    let legacy = source.legacy_rotation.is_some();
                    Some((
                        crate::skull_skin::key(profile, legacy),
                        (profile.clone(), legacy),
                    ))
                })
                .collect::<BTreeMap<_, _>>();
            if self
                .head_results
                .select(wanted.keys().copied().collect(), head_cost)
            {
                self.skin_revision = self.skin_revision.wrapping_add(1);
            }
            self.head_queue
                .retain(|(key, _, _)| wanted.contains_key(key));
            let queued = self
                .head_queue
                .iter()
                .map(|(key, _, _)| *key)
                .collect::<BTreeSet<_>>();
            for (key, (profile, legacy)) in wanted {
                if !self.head_results.contains(&key)
                    && !queued.contains(&key)
                    && self.head_inflight != Some((self.head_epoch, key))
                {
                    self.head_queue.push_back((key, profile, legacy));
                }
            }
            self.head_snapshot = Some((Arc::clone(snapshot), bank_identity));
        }
        if self.head_inflight.is_some() {
            return Task::none();
        }
        let Some((key, profile, legacy)) = self.head_queue.pop_front() else {
            return Task::none();
        };
        let epoch = self.head_epoch;
        let online = self.online_skins;
        let stop = Arc::clone(&self.head_cancelled);
        self.head_inflight = Some((epoch, key));
        self.head_requests += 1;
        Task::perform(
            crate::nbt_background(move || {
                let sources = crate::skull_skin::Sources::local(online)?;
                crate::skull_skin::load(&profile, legacy, &sources, &|| {
                    stop.load(Ordering::Acquire)
                })
                .map(Arc::new)
            }),
            move |result| Message::HeadSkinLoaded(epoch, key, result),
        )
    }
    pub fn accept_head_skin(
        &mut self,
        epoch: u64,
        key: crate::skull_skin::Key,
        result: Result<Arc<crate::skull_skin::Appearance>, String>,
    ) {
        if self.head_inflight != Some((epoch, key)) {
            return;
        }
        self.head_inflight = None;
        if epoch != self.head_epoch || self.head_context.is_none() {
            return;
        }
        if let Err(error) = &result {
            log::error!("[skulls] 头颅外观准备失败：{error}");
        }
        let cost = head_cost(&result);
        if self.head_results.publish(key, result, cost) {
            self.skin_revision = self.skin_revision.wrapping_add(1);
        }
    }
    fn maintain_skins(
        &mut self,
        snapshot: Option<&Arc<Snapshot>>,
        root: Option<&std::path::PathBuf>,
        active: bool,
    ) -> Task<Message> {
        let context = root.map(|root| (root.clone(), self.online_skins));
        if self.skin_context != context {
            self.skin_context = context;
            self.skin_epoch = self.skin_epoch.wrapping_add(1);
            self.skin_revision = self.skin_revision.wrapping_add(1);
            self.skin_results.clear();
            self.skin_queue.clear();
            self.skin_queued.clear();
            self.skin_snapshot = None;
        }
        let available = self.skin_context.is_some() && self.enabled && self.online_skins && active;
        if !available
            && self
                .skin_inflight
                .is_some_and(|(epoch, _)| epoch == self.skin_epoch)
        {
            self.skin_epoch = self.skin_epoch.wrapping_add(1);
        }
        let Some(snapshot) = snapshot.filter(|_| available) else {
            if self.skin_snapshot.take().is_some() {
                self.skin_queue.clear();
                self.skin_queued.clear();
                self.skin_results.select(BTreeSet::new(), skin_cost);
                self.skin_revision = self.skin_revision.wrapping_add(1);
            }
            return Task::none();
        };
        if self
            .skin_snapshot
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, snapshot))
        {
            let wanted = snapshot
                .markers
                .iter()
                .filter(|m| m.marker().kind == Kind::Player)
                .filter_map(|m| player_uuid(m.marker()))
                .collect::<BTreeSet<_>>();
            if self.skin_results.select(wanted.clone(), skin_cost) {
                self.skin_revision = self.skin_revision.wrapping_add(1);
            }
            self.skin_queue.retain(|uuid| wanted.contains(uuid));
            let inflight = self.skin_inflight;
            self.skin_queued.retain(|uuid| {
                wanted.contains(uuid) || inflight.is_some_and(|(_, active)| active == *uuid)
            });
            for uuid in wanted {
                if !self.skin_results.contains(&uuid) && self.skin_queued.insert(uuid) {
                    self.skin_queue.push_back(uuid);
                }
            }
            self.skin_snapshot = Some(Arc::clone(snapshot));
        }
        if self.skin_inflight.is_some() {
            return Task::none();
        }
        let Some(uuid) = self.skin_queue.pop_front() else {
            return Task::none();
        };
        let epoch = self.skin_epoch;
        self.skin_inflight = Some((epoch, uuid));
        self.skin_requests += 1;
        Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    if tx
                        .send(crate::player_skin::load(uuid).map(Arc::new))
                        .is_err()
                    {
                        log::debug!("[players] 皮肤响应接收端已关闭");
                    }
                });
                rx.await
                    .unwrap_or_else(|e| Err(format!("皮肤后台请求未返回：{e}")))
            },
            move |result| Message::PlayerSkinLoaded(epoch, uuid, result),
        )
    }
    pub fn accept_skin(
        &mut self,
        epoch: u64,
        uuid: [u8; 16],
        result: Result<Arc<crate::player_skin::Appearance>, String>,
    ) {
        if self.skin_inflight != Some((epoch, uuid)) {
            return;
        }
        self.skin_inflight = None;
        if epoch != self.skin_epoch || self.skin_context.is_none() {
            return;
        }
        if let Err(error) = &result {
            log::error!(
                "[players] 在线皮肤 {} 失败：{error}",
                mcedit2_world::player_catalog::uuid_string(uuid)
            );
        }
        let cost = skin_cost(&result);
        if self.skin_results.publish(uuid, result, cost) {
            self.skin_revision = self.skin_revision.wrapping_add(1);
        }
    }
    #[cfg(test)]
    pub fn maintain(
        &mut self,
        snapshot: Option<Arc<Snapshot>>,
        bank: Option<Arc<TextureBank>>,
        version: Option<i32>,
        show_textures: bool,
        active: bool,
    ) -> Task<Message> {
        self.maintain_with_root(
            snapshot,
            bank,
            version,
            show_textures,
            active,
            Some(std::path::PathBuf::new()),
        )
    }
    pub fn maintain_with_root(
        &mut self,
        snapshot: Option<Arc<Snapshot>>,
        bank: Option<Arc<TextureBank>>,
        version: Option<i32>,
        show_textures: bool,
        active: bool,
        root: Option<std::path::PathBuf>,
    ) -> Task<Message> {
        let retain = self
            .geometry_cache
            .as_ref()
            .zip(bank.as_deref())
            .zip(root.as_deref())
            .is_some_and(|((frame, bank), root)| {
                scene::cache_matches_context(
                    frame,
                    scene::Context {
                        bank,
                        version,
                        root,
                        maps: self.maps.as_ref(),
                        language: self.language,
                        textures: show_textures,
                        entities: self.enabled,
                        online: self.online_skins,
                        dimension: snapshot.as_ref().map(|s| s.key),
                        item_world: self.item_world.as_ref(),
                    },
                )
            });
        if !retain {
            self.geometry_cache = None;
        }
        let head_task =
            self.maintain_heads(snapshot.as_ref(), bank.as_ref(), root.as_ref(), active);
        let skin_task = Task::batch([
            self.maintain_skins(snapshot.as_ref(), root.as_ref(), active),
            head_task,
        ]);
        let next =
            snapshot
                .zip(bank)
                .zip(root)
                .filter(|_| active)
                .map(|((snapshot, bank), root)| Input {
                    selected: self.selected.clone(),
                    item_world: self.item_world.clone(),
                    language: self.language,
                    snapshot,
                    bank,
                    version,
                    show_textures,
                    show_entities: self.enabled,
                    visibility: self.visibility,
                    root,
                    maps: self.maps.clone(),
                    online_skins: self.online_skins,
                    skin_revision: self.skin_revision,
                    skin_results: Arc::clone(self.skin_results.active()),
                    head_results: Arc::clone(self.head_results.active()),
                });
        let Some(next) = next else {
            if self.request.take().is_some() {
                self.cancelled.store(true, Ordering::Release);
                self.epoch = self.epoch.wrapping_add(1);
                self.frame = None;
                *self.composite.lock().expect("实体合成缓存锁异常") = None;
                self.status.clear();
            }
            return skin_task;
        };
        if self
            .geometry_cache
            .as_ref()
            .is_some_and(|frame| !scene::cache_matches(frame, &next))
        {
            self.geometry_cache = None;
        }
        if self.running {
            if self
                .request
                .as_ref()
                .is_some_and(|r| !r.same_geometry(&next))
            {
                self.cancelled.store(true, Ordering::Release);
                self.epoch = self.epoch.wrapping_add(1);
                self.request = None;
                self.status = "正在取消过期的实体与皮肤请求…".into();
            }
            return skin_task;
        }
        if self.request.as_ref().is_some_and(|r| r.same(&next)) {
            return skin_task;
        }
        self.cancelled.store(true, Ordering::Release);
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.epoch = self.epoch.wrapping_add(1);
        let epoch = self.epoch;
        self.request = Some(next.clone());
        self.running = true;
        self.status = "正在后台编译静态生物模型…".into();
        let cancelled = Arc::clone(&self.cancelled);
        let previous = self.geometry_cache.clone();
        let model_task = Task::perform(
            async move {
                let (tx, rx) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = scene::assemble(
                        next.snapshot,
                        &next.bank,
                        next.version,
                        next.show_textures,
                        next.show_entities,
                        &|| cancelled.load(Ordering::Acquire),
                        Some(&next.root),
                        next.online_skins,
                        &next.skin_results,
                        &next.head_results,
                        next.language,
                        previous.as_ref(),
                        next.item_world.as_ref(),
                        next.selected.as_ref(),
                        next.visibility,
                        next.maps.as_ref(),
                    )
                    .map(Arc::new);
                    if tx.send(result).is_err() {
                        log::debug!("[entities] 模型结果接收端已关闭");
                    }
                });
                (
                    epoch,
                    rx.await
                        .unwrap_or_else(|e| Err(format!("实体模型工作线程未返回：{e}"))),
                )
            },
            |(epoch, result)| Message::EntityModelsLoaded(epoch, result),
        );
        Task::batch([skin_task, model_task])
    }
    pub fn accept(&mut self, epoch: u64, result: Result<Arc<Frame>, String>) {
        self.running = false;
        if epoch != self.epoch {
            return;
        }
        match result {
            Ok(frame) => {
                let previous_diagnostics = self
                    .frame
                    .as_ref()
                    .map(|old| {
                        old.diagnostics
                            .iter()
                            .map(String::as_str)
                            .collect::<BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                for diagnostic in &frame.diagnostics {
                    if !previous_diagnostics.contains(diagnostic.as_str()) {
                        log::error!("[entities] {diagnostic}");
                    }
                }
                if let Some(assembly) = &frame.assembly {
                    self.scene_totals.add(assembly.statistics);
                }
                self.status = format!(
                    "方块实体、模型与地图：{} 已建模 / {} 缺失 / {} 不可见 · 玩家皮肤：{}",
                    frame.rendered,
                    frame.missing,
                    frame.invisible,
                    if self.online_skins {
                        "在线 / 缓存"
                    } else {
                        "离线默认"
                    }
                );
                self.geometry_cache = Some(Arc::clone(&frame));
                self.frame = Some(frame);
            }
            Err(error) => {
                log::error!("[entities] {error}");
                self.status = error;
                self.frame = None;
            }
        }
    }
}

/// 只复用同一图集的实体与地形；保留基础 Arc 供流式调度判断真正的地形交接。
pub(crate) fn compose(
    base: &Arc<StreamMesh>,
    frame: Option<&Arc<Frame>>,
    cache: &Mutex<Option<Arc<StreamMesh>>>,
) -> Arc<StreamMesh> {
    let terrain = base.terrain.as_ref().unwrap_or(base);
    let Some(frame) = frame else {
        return Arc::clone(terrain);
    };
    let mut cache = cache.lock().expect("实体合成缓存锁异常");
    if let Some(old) = cache.as_ref().filter(|old| {
        old.terrain
            .as_ref()
            .is_some_and(|t| Arc::ptr_eq(t, terrain))
            && old.entities.as_ref().is_some_and(|f| Arc::ptr_eq(f, frame))
    }) {
        return Arc::clone(old);
    }
    if terrain
        .atlas
        .as_ref()
        .is_none_or(|a| !crate::textures::atlas_compatible(&frame.atlas, a))
    {
        return Arc::clone(base);
    }
    let chunks = terrain.chunks.join(&frame.chunks);
    let result = Arc::new(StreamMesh {
        complete: terrain.complete,
        chunks,
        origin: terrain.origin,
        atlas: Some(Arc::clone(&frame.atlas)),
        terrain: Some(Arc::clone(terrain)),
        visibility: terrain.visibility.clone(),
        entities: Some(Arc::clone(frame)),
    });
    *cache = Some(Arc::clone(&result));
    result
}

#[cfg(test)]
#[path = "banner_registry_tests.rs"]
mod banner_registry_tests;
#[cfg(test)]
#[path = "frame_tests.rs"]
mod frame_tests;
#[cfg(test)]
#[path = "hanging_tests.rs"]
mod hanging_tests;
#[cfg(test)]
#[path = "item_world_tests.rs"]
mod item_world_tests;
#[cfg(test)]
#[path = "map_geometry_tests.rs"]
pub(crate) mod map_tests;
#[cfg(test)]
#[path = "painting_tests.rs"]
mod painting_tests;
#[cfg(test)]
#[path = "piston_render_tests.rs"]
mod piston_tests;
#[cfg(test)]
#[path = "entity_geometry_tests.rs"]
pub(crate) mod tests;
#[cfg(test)]
#[path = "entity_versions_tests.rs"]
mod version_tests;

#[test]
fn test_banner_registry_tests_complete() {
    banner_registry_tests::complete();
}

#[test]
fn test_piston_tests_complete() {
    piston_tests::complete();
}

#[test]
fn test_painting_tests_complete() {
    painting_tests::complete();
}

#[test]
fn test_hanging_tests_complete() {
    hanging_tests::complete();
}

#[test]
fn test_frame_tests_complete() {
    frame_tests::complete();
}

#[test]
fn test_item_world_tests_complete() {
    item_world_tests::complete();
}

#[test]
fn test_map_tests_complete() {
    map_tests::complete();
}

#[test]
fn test_version_tests_run() {
    version_tests::run();
}
