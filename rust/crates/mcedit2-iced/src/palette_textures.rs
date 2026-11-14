//! Java atlas 的调色板排列来源；按覆盖后的真实 PNG 生成纹饰，并保留源动画。
use crate::textures::texture_animation::Sprite;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Default)]
pub(super) struct Definitions {
    recipes: BTreeMap<String, [String; 3]>,
    bytes: usize,
}
pub(super) fn asset(name: &str) -> bool {
    name.strip_prefix("assets/")
        .and_then(|p| p.split_once('/'))
        .is_some_and(|(_, p)| p.starts_with("atlases/") && p.ends_with(".json"))
}
fn id(value: &serde_json::Value) -> Result<String, String> {
    crate::resource_ids::qualified(
        value.as_str().ok_or("atlas 纹理 ID 必须为字符串")?,
        "minecraft",
    )
    .ok_or("atlas 纹理 ID 无效".into())
}
impl Definitions {
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn insert(
        &mut self,
        bytes: &[u8],
        mut check: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<(), String> {
        let doc: serde_json::Value = mcedit2_world::client_json::object(bytes)?;
        let sources = doc
            .get("sources")
            .and_then(|v| v.as_array())
            .ok_or("atlas 定义缺少 sources 数组")?;
        for source in sources {
            let kind = source
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or("atlas 来源缺少 type")?;
            if !matches!(
                kind,
                "paletted_permutations" | "minecraft:paletted_permutations"
            ) {
                continue;
            }
            let key = id(source
                .get("palette_key")
                .ok_or("调色板来源缺少 palette_key")?)?;
            let variants = source
                .get("permutations")
                .and_then(|v| v.as_object())
                .ok_or("调色板来源缺少 permutations")?;
            for texture in source
                .get("textures")
                .and_then(|v| v.as_array())
                .ok_or("调色板来源缺少 textures")?
            {
                let texture = id(texture)?;
                for (suffix, palette) in variants {
                    let output = format!("{texture}_{suffix}");
                    crate::resource_ids::qualified(&output, "minecraft")
                        .ok_or("调色板输出 ID 无效")?;
                    let palette = id(palette)?;
                    let previous = self.recipes.get(&output).map_or(0, |recipe| {
                        output.len() + recipe.iter().map(String::len).sum::<usize>() + 192
                    });
                    let next = self
                        .bytes
                        .checked_sub(previous)
                        .and_then(|n| {
                            n.checked_add(
                                output.len() + texture.len() + key.len() + palette.len() + 192,
                            )
                        })
                        .ok_or("调色板定义大小溢出")?;
                    check(next)?;
                    self.recipes
                        .insert(output, [texture.clone(), key.clone(), palette]);
                    self.bytes = next;
                }
            }
        }
        Ok(())
    }
    pub fn equipment_outputs(&self) -> BTreeSet<String> {
        self.recipes
            .keys()
            .filter(|id| crate::armor_geometry::texture_dependency(id))
            .cloned()
            .collect()
    }
    pub fn outputs(&self, wanted: &BTreeSet<String>) -> BTreeSet<String> {
        self.recipes
            .keys()
            .filter(|id| wanted.contains(*id))
            .cloned()
            .collect()
    }
    pub fn dependencies(&self, wanted: &BTreeSet<String>) -> BTreeSet<String> {
        self.recipes
            .iter()
            .filter(|(id, _)| wanted.contains(*id))
            .flat_map(|(_, recipe)| recipe.iter().cloned())
            .collect()
    }
    pub fn apply(
        &self,
        bank: &mut crate::textures::TextureBank,
        wanted: &BTreeSet<String>,
        memory: &mut usize,
        budget: crate::resource_budget::ResourceBudget,
        control: &super::LoadControl,
    ) -> Result<(), String> {
        for (output, recipe) in self.recipes.iter().filter(|(id, _)| wanted.contains(*id)) {
            control.check()?;
            let [base, key, palette] = recipe;
            let key = bank
                .get(key)
                .ok_or_else(|| format!("调色板键纹理不存在：{key}"))?;
            let palette = bank
                .get(palette)
                .ok_or_else(|| format!("调色板纹理不存在：{palette}"))?;
            if key.rgba.len() != palette.rgba.len() {
                return Err(format!("调色板 {output} 的键和值长度不同"));
            }
            let mut mapping = BTreeMap::new();
            for (key, color) in key.rgba.chunks_exact(4).zip(palette.rgba.chunks_exact(4)) {
                if key[3] != 0 {
                    mapping.insert(
                        [key[0], key[1], key[2]],
                        [color[0], color[1], color[2], color[3]],
                    );
                }
            }
            let animated = bank.live_animations.get(base).cloned();
            let source = match &animated {
                Some(sprite) => &sprite.source,
                None => bank
                    .get(base)
                    .ok_or_else(|| format!("调色板源纹理不存在：{base}"))?,
            };
            let additional = source.rgba.len()
                + animated.as_ref().map_or(0, |s| {
                    s.sequence.width as usize * s.sequence.height as usize * 4
                        + s.sequence.steps.len()
                            * std::mem::size_of::<crate::textures::texture_animation::Step>()
                });
            budget.check(
                bank.models.source_bytes(),
                memory
                    .checked_add(additional + mapping.len() * 64 + self.bytes())
                    .ok_or("调色板工作集溢出")?,
                0,
            )?;
            let mut tile = source.clone();
            for (index, pixel) in tile.rgba.chunks_exact_mut(4).enumerate() {
                if index % 4096 == 0 {
                    control.check()?;
                }
                if pixel[3] == 0 {
                    continue;
                }
                if let Some(color) = mapping.get(&[pixel[0], pixel[1], pixel[2]]) {
                    let alpha = (u32::from(pixel[3]) * u32::from(color[3]) / 255) as u8;
                    pixel.copy_from_slice(color);
                    pixel[3] = alpha;
                }
            }
            let (tile, sprite) = match animated {
                Some(s) => {
                    let sprite = Arc::new(Sprite {
                        source: tile,
                        sequence: s.sequence.clone(),
                    });
                    (sprite.tile(sprite.sequence.stamp(0)), Some(sprite))
                }
                None => (tile, None),
            };
            let rgba = tile.rgba.len() + sprite.as_ref().map_or(0, |s| s.source.rgba.len());
            let old = Arc::make_mut(&mut bank.tiles).insert(output.clone(), tile);
            let old_sprite = match sprite {
                Some(sprite) => {
                    Arc::make_mut(&mut bank.live_animations).insert(output.clone(), sprite)
                }
                None => Arc::make_mut(&mut bank.live_animations).remove(output),
            };
            let removed_rgba = old.as_ref().map_or(0, |t| t.rgba.len())
                + old_sprite.as_ref().map_or(0, |s| s.source.rgba.len());
            let statistics = Arc::make_mut(&mut bank.statistics);
            statistics.decoded_bytes = statistics
                .decoded_bytes
                .checked_add(rgba)
                .and_then(|n| n.checked_sub(removed_rgba))
                .ok_or("调色板 RGBA 计量溢出")?;
            *memory = memory
                .checked_add(additional)
                .and_then(|n| {
                    n.checked_sub(
                        old.map_or(0, |t| t.rgba.len()) + old_sprite.map_or(0, |s| s.bytes()),
                    )
                })
                .ok_or("调色板常驻计量溢出")?;
        }
        Ok(())
    }
}
