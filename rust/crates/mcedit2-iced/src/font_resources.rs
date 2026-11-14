//! 字体源随实际资源包顺序保存；同名字体定义合并，二进制和图片由高优先级包覆盖。
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct Resources {
    pub(crate) unihex: Arc<crate::font_unihex::Cache>,
    pub(crate) legacy_112: bool,
    definitions: BTreeMap<String, Vec<Arc<Vec<u8>>>>,
    binaries: BTreeMap<String, Arc<Vec<u8>>>,
    // Font provider 直接读取整张 PNG，不应用纹理动画元数据或模型材质处理。
    images: BTreeMap<String, Arc<crate::textures::Tile>>,
    bytes: usize,
}

pub(crate) fn asset(name: &str) -> Option<(String, bool)> {
    let mut parts = name.split('/');
    if parts.next()? != "assets" {
        return None;
    }
    let namespace = parts.next()?;
    if !crate::resource_ids::namespace(namespace) || parts.next()? != "font" {
        return None;
    }
    let path = parts.collect::<Vec<_>>().join("/");
    if !crate::resource_ids::path(&path) {
        return None;
    }
    if let Some(path) = path.strip_suffix(".json") {
        return Some((format!("{namespace}:{path}"), true));
    }
    [".zip", ".hex", ".ttf", ".otf", ".bin"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
        .then(|| (format!("{namespace}:font/{path}"), false))
}

impl Resources {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn insert(
        &mut self,
        id: String,
        definition: bool,
        bytes: Vec<u8>,
    ) -> Result<(), String> {
        if definition {
            let doc = mcedit2_world::client_json::object(&bytes)
                .map_err(|e| format!("字体 {id} JSON 无效：{e}"))?;
            if !doc
                .get("providers")
                .is_some_and(serde_json::Value::is_array)
            {
                return Err(format!("字体 {id} 缺少 providers 数组"));
            }
            let cost = bytes
                .len()
                .checked_add(id.len())
                .and_then(|n| n.checked_add(128))
                .ok_or("字体来源大小溢出")?;
            self.bytes = self.bytes.checked_add(cost).ok_or("字体来源总量溢出")?;
            self.definitions
                .entry(id)
                .or_default()
                .push(Arc::new(bytes));
        } else {
            let previous = self
                .binaries
                .get(&id)
                .map_or(0, |value| value.len() + id.len() + 128);
            self.bytes = self
                .bytes
                .checked_sub(previous)
                .and_then(|n| n.checked_add(bytes.len()))
                .and_then(|n| n.checked_add(id.len() + 128))
                .ok_or("字体二进制来源大小溢出")?;
            self.binaries.insert(id, Arc::new(bytes));
        }
        Ok(())
    }

    pub(crate) fn definitions(&self, id: &str) -> Option<&[Arc<Vec<u8>>]> {
        self.definitions.get(id).map(Vec::as_slice)
    }
    pub(crate) fn binary(&self, id: &str) -> Option<&Arc<Vec<u8>>> {
        self.binaries.get(id)
    }
    pub(crate) fn image(&self, id: &str) -> Option<&crate::textures::Tile> {
        self.images
            .get(&crate::resource_models::normalize_texture(id)?)
            .map(Arc::as_ref)
    }
    pub(crate) fn insert_image(&mut self, id: String, image: crate::textures::Tile) {
        self.images.insert(id, Arc::new(image));
    }
    pub(crate) fn image_ids(&self) -> impl Iterator<Item = &String> {
        self.images.keys()
    }

    /// 即使只加载方块图片，字体位图仍须进入同一资源依赖集合。
    pub(crate) fn texture_dependencies(&self) -> Result<BTreeSet<String>, String> {
        let mut textures = BTreeSet::new();
        if self.legacy_112 {
            textures.insert("minecraft:font/ascii".into());
            if let Some(sizes) = self.binaries.get("minecraft:font/glyph_sizes.bin") {
                if sizes.len() != 65536 {
                    return Err("1.12 glyph_sizes.bin 必须包含 65536 项宽度".into());
                }
                for (page, values) in sizes.chunks_exact(256).enumerate() {
                    if values.iter().any(|size| *size != 0) {
                        textures.insert(format!("minecraft:font/unicode_page_{page:02x}"));
                    }
                }
            }
        }
        for layers in self.definitions.values() {
            for layer in layers {
                let doc: serde_json::Value = mcedit2_world::client_json::object(layer)?;
                for provider in doc["providers"].as_array().ok_or("字体 providers 无效")? {
                    if provider["type"]
                        .as_str()
                        .is_some_and(|kind| kind == "bitmap" || kind == "minecraft:bitmap")
                    {
                        let file = provider["file"]
                            .as_str()
                            .ok_or("字体位图 provider 缺少 file")?;
                        let id = crate::resource_ids::qualified(file, "minecraft")
                            .ok_or("字体位图资源 ID 无效")?;
                        let id = id.strip_suffix(".png").ok_or("字体位图必须引用 PNG")?;
                        textures.insert(
                            crate::resource_models::normalize_texture(id)
                                .ok_or("字体位图纹理 ID 无效")?,
                        );
                    }
                }
            }
        }
        Ok(textures)
    }
}
