//! 当前场景的动态页集合：保留仍活动的页号，释放离场页，旧画面持有自己的不可变页。
use super::{AtlasPage, AtlasUv, RuntimeTexture, TextureAtlas};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct Page {
    pub mip: Arc<crate::material_mips::MipPage>,
    pub uv: AtlasUv,
    pub bytes: u64,
}
#[derive(Clone)]
struct Material {
    texture: Arc<RuntimeTexture>,
    page: Page,
}
pub(crate) struct Set {
    pub atlas: Arc<TextureAtlas>,
    pub base: Arc<TextureAtlas>,
    materials: BTreeMap<String, Material>,
}
#[derive(Clone, Copy, Default, serde::Serialize)]
pub(crate) struct Statistics {
    pub built: usize,
    pub reused: usize,
}
fn same(a: &Arc<RuntimeTexture>, b: &Arc<RuntimeTexture>) -> bool {
    Arc::ptr_eq(a, b) || a.same(b)
}
/// 所需素材不设数量上限；预算按活动页实际像素及 mip 计算。
pub(crate) fn reconcile(
    base: Arc<TextureAtlas>,
    mut textures: BTreeMap<String, Arc<RuntimeTexture>>,
    previous: Option<&Arc<Set>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Arc<Set>, Statistics), String> {
    let check = || {
        if cancelled() {
            Err("运行时图集合成已取消".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let mut inherited = Vec::new();
    for (id, texture) in &textures {
        if id.is_empty() || id != &texture.id {
            return Err(format!("运行时贴图 ID 无效：{id}"));
        }
        if base.entries.contains_key(id) {
            let old = base
                .runtime_sources
                .get(id)
                .ok_or_else(|| format!("运行时贴图与资源冲突：{id}"))?;
            if !same(old, texture) {
                return Err(format!("同一运行时材质 ID 对应不同源：{id}"));
            }
            inherited.push(id.clone());
        }
    }
    for id in inherited {
        textures.remove(&id);
    }
    let previous = previous.filter(|old| Arc::ptr_eq(&old.base, &base));
    if let Some(old) = previous.filter(|old| {
        old.materials.len() == textures.len()
            && textures.iter().all(|(id, texture)| {
                old.materials
                    .get(id)
                    .is_some_and(|material| same(texture, &material.texture))
            })
    }) {
        return Ok((
            Arc::clone(old),
            Statistics {
                built: 0,
                reused: textures.len(),
            },
        ));
    }
    if textures.is_empty() {
        return Ok((
            Arc::new(Set {
                atlas: Arc::clone(&base),
                base,
                materials: BTreeMap::new(),
            }),
            Statistics::default(),
        ));
    }
    let first = base.pages.len();
    let count = first
        .checked_add(textures.len())
        .ok_or("运行时图集页数溢出")?;
    u32::try_from(count).map_err(|_| "运行时图集页数超出索引范围")?;
    let mut slots = BTreeMap::new();
    let mut occupied = vec![false; textures.len()];
    if let Some(old) = previous {
        for id in textures.keys() {
            if let Some(uv) = old.atlas.entries.get(id) {
                let index = uv.page as usize;
                if (first..count).contains(&index) {
                    occupied[index - first] = true;
                    slots.insert(id.clone(), index);
                }
            }
        }
    }
    let mut free = occupied
        .iter()
        .enumerate()
        .filter_map(|(i, used)| (!used).then_some(first + i));
    for id in textures.keys() {
        if !slots.contains_key(id) {
            slots.insert(
                id.clone(),
                free.next().expect("运行时空闲槽必须覆盖未分配素材"),
            );
        }
    }
    let mut materials = BTreeMap::new();
    let mut allocated = base.allocated_bytes;
    let mut statistics = Statistics::default();
    // 先计入全部复用页，再为新增页分配工作内存。
    if let Some(old) = previous {
        for (id, texture) in &textures {
            check()?;
            if let Some(material) = old.materials.get(id).filter(|m| same(texture, &m.texture)) {
                allocated = allocated
                    .checked_add(material.page.bytes)
                    .ok_or("运行时图集预算溢出")?;
                if allocated > base.budget_bytes {
                    return Err("运行时图集页超出资源预算".into());
                }
                materials.insert(id.clone(), material.clone());
                statistics.reused += 1;
            }
        }
    }
    for (id, texture) in textures {
        check()?;
        if !materials.contains_key(&id) {
            let page = build_page(&texture, allocated, base.budget_bytes, cancelled)?;
            allocated += page.bytes;
            materials.insert(id, Material { texture, page });
            statistics.built += 1;
        }
    }
    let mut atlas = (*base).clone();
    let mut pages = vec![None; count - first];
    for (id, material) in &materials {
        let page = slots[id];
        pages[page - first] = Some(&material.page);
        Arc::make_mut(&mut atlas.entries).insert(
            id.clone(),
            AtlasUv {
                page: page as u32,
                ..material.page.uv
            },
        );
        Arc::make_mut(&mut atlas.entries).insert(
            super::simple_key(id),
            AtlasUv {
                page: page as u32,
                ..material.page.uv
            },
        );
    }
    for page in pages {
        let page = page.expect("运行时页计划必须完整");
        atlas.pages.push(Arc::clone(&page.mip.base));
        atlas.mip_pages.push(Arc::clone(&page.mip));
    }
    atlas.allocated_bytes = allocated;
    for (id, material) in &materials {
        Arc::make_mut(&mut atlas.runtime_sources).insert(id.clone(), Arc::clone(&material.texture));
    }
    check()?;
    Ok((
        Arc::new(Set {
            atlas: Arc::new(atlas),
            base,
            materials,
        }),
        statistics,
    ))
}

pub(crate) fn build_page(
    texture: &RuntimeTexture,
    allocated_bytes: u64,
    budget_bytes: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Page, String> {
    if texture.width == 0 || texture.height == 0 {
        return Err("运行时贴图尺寸为零".into());
    }
    let expected = (texture.width as usize)
        .checked_mul(texture.height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or("运行时贴图尺寸溢出")?;
    if texture.rgba.len() != expected {
        return Err(format!("运行时贴图 {} 的 RGBA 字节数不匹配", texture.id));
    }
    let width = texture.width.checked_add(2).ok_or("运行时贴图宽度溢出")?;
    let height = texture.height.checked_add(2).ok_or("运行时贴图高度溢出")?;
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|v| v.checked_mul(4))
        .ok_or("运行时图集页大小溢出")?;
    let page_bytes = u64::try_from(len).map_err(|_| "运行时图集页大小溢出")?;
    let binding_bytes = super::binding_bytes(&texture.id)
        .checked_add(super::binding_bytes(&super::simple_key(&texture.id)))
        .ok_or("运行时采样身份预算溢出")?;
    let used = allocated_bytes
        .checked_add(page_bytes)
        .and_then(|bytes| bytes.checked_add(binding_bytes))
        .ok_or("运行时图集预算溢出")?;
    let remaining = budget_bytes
        .checked_sub(used)
        .ok_or("运行时图集页超出资源预算")?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(len)
        .map_err(|e| format!("运行时图集内存分配失败：{e}"))?;
    rgba.resize(len, 0);
    for y in 0..height {
        if cancelled() {
            return Err("运行时图集合成已取消".into());
        }
        for x in 0..width {
            let sx = x.saturating_sub(1).min(texture.width - 1);
            let sy = y.saturating_sub(1).min(texture.height - 1);
            let src = (sy as usize * texture.width as usize + sx as usize) * 4;
            let dst = (y as usize * width as usize + x as usize) * 4;
            rgba[dst..dst + 4].copy_from_slice(&texture.rgba[src..src + 4]);
        }
    }
    let base = Arc::new(AtlasPage {
        width,
        height,
        rgba,
    });
    let rect = crate::material_mips::SpriteRect {
        x: 1,
        y: 1,
        width: texture.width,
        height: texture.height,
    };
    let (mut mip, cost) =
        crate::material_mips::build(Arc::clone(&base), &[rect], remaining, &mut || {
            if cancelled() {
                Err("运行时图集合成已取消".into())
            } else {
                Ok(())
            }
        })?;
    Arc::get_mut(&mut mip)
        .expect("新运行时 mip 尚未共享")
        .materials[0]
        .padding[0] = texture.sampler.map_or(0, |sampler| sampler.flags(true));
    Ok(Page {
        mip,
        bytes: page_bytes
            .checked_add(cost)
            .and_then(|bytes| bytes.checked_add(binding_bytes))
            .ok_or("运行时 mip 预算溢出")?,
        uv: AtlasUv {
            page: 0,
            start: [1.5 / width as f32, 1.5 / height as f32],
            end: [
                (texture.width as f32 + 0.5) / width as f32,
                (texture.height as f32 + 0.5) / height as f32,
            ],
        },
    })
}
