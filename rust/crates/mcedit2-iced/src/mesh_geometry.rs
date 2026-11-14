//! 不可变的原始几何与渲染通道分类一起冻结，实例位置、拾取和 GPU 编码另行持有。
use crate::world_mesh::WorldQuad;
use std::{ops::Range, sync::Arc};

#[derive(Debug)]
pub(crate) struct Geometry {
    quads: Arc<Vec<WorldQuad>>,
    regular: Vec<Range<usize>>,
    ordered: Vec<usize>,
    blended: Arc<Vec<usize>>,
    light_usage: crate::lighting::Usage,
    used_pages: Arc<[u32]>,
    additive: bool,
    biome_dependent: bool,
}
impl Geometry {
    pub(crate) fn new(quads: Arc<Vec<WorldQuad>>) -> Self {
        Self::with_biomes(quads, true)
    }
    /// 由实际几何生成路径记录颜色依赖；其他来源继续保留完整依赖。
    pub(crate) fn with_biomes(quads: Arc<Vec<WorldQuad>>, biome_dependent: bool) -> Self {
        let mut light_usage = crate::lighting::Usage::default();
        let mut regular: Vec<Range<usize>> = Vec::new();
        let mut ordered = Vec::new();
        let mut blended = Vec::new();
        let mut additive = false;
        let mut used_pages = Vec::new();
        for (i, quad) in quads.iter().enumerate() {
            light_usage.observe(quad.light);
            if !used_pages.contains(&quad.atlas_page) {
                used_pages.push(quad.atlas_page);
            }
            additive |= quad.mode & crate::material_mode::ADDITIVE != 0;
            if quad.blended() {
                blended.push(i);
            } else if quad.ordered_opaque() {
                ordered.push(i);
            } else if let Some(last) = regular.last_mut().filter(|last| last.end == i) {
                last.end += 1;
            } else {
                regular.push(i..i + 1);
            }
        }
        Self {
            quads,
            light_usage,
            used_pages: used_pages.into(),
            additive,
            biome_dependent,
            regular,
            ordered,
            blended: Arc::new(blended),
        }
    }
    /// 几何真正引用的原始材质页；运行时其他对象页的增删不应使本几何失效。
    #[inline]
    pub(crate) fn used_pages(&self) -> &Arc<[u32]> {
        &self.used_pages
    }
    pub(crate) fn biome_dependent(&self) -> bool {
        self.biome_dependent
    }
    pub(crate) fn lighting_usage(&self) -> crate::lighting::Usage {
        self.light_usage
    }
    pub(crate) fn has_additive(&self) -> bool {
        self.additive
    }
    #[inline]
    pub(crate) fn quads(&self) -> &Arc<Vec<WorldQuad>> {
        &self.quads
    }
    #[inline]
    pub(crate) fn ordered(&self) -> &[usize] {
        &self.ordered
    }
    #[inline]
    pub(crate) fn blended(&self) -> &Arc<Vec<usize>> {
        &self.blended
    }
    pub(crate) fn has_regular(&self) -> bool {
        !self.regular.is_empty()
    }
    pub(crate) fn regular(&self) -> impl Iterator<Item = usize> + '_ {
        self.regular.iter().cloned().flatten()
    }
}
