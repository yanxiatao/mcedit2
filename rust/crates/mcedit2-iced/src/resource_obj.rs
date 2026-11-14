//! NeoForge / Forge 声明式 OBJ 烘焙；所有引用只查询已载入资源索引。
use super::{graph::MeshSpec, normalize_texture, ModelBank, ObjSelection};
use crate::{isometric::Face, resource_ids};

/// NeoForge 的 OBJ 声明也可引用保留了打包后缀的 Wavefront 文件。
pub(crate) fn obj_source_suffix(path: &str) -> Option<&'static str> {
    [".obj", ".obj.ie"]
        .into_iter()
        .find(|suffix| path.ends_with(*suffix))
}

pub(super) fn asset(path: &str, suffix: &str) -> Option<String> {
    let mut parts = path.splitn(3, '/');
    if parts.next()? != "assets" {
        return None;
    }
    let namespace = parts.next()?;
    let path = parts.next()?;
    (path.starts_with("models/") && path.ends_with(suffix))
        .then(|| resource_ids::qualified(&format!("{namespace}:{path}"), "minecraft"))?
}

fn library(model: &str, raw: &str) -> Result<String, String> {
    let (namespace, path) = model.split_once(':').ok_or("OBJ 资源 ID 无效")?;
    let reference = if raw.contains(':') {
        raw.to_owned()
    } else {
        format!(
            "{namespace}:{}{raw}",
            path.rsplit_once('/')
                .map_or("", |(p, _)| &path[..p.len() + 1])
        )
    };
    resource_ids::qualified(&reference, "minecraft")
        .filter(|id| id.ends_with(".mtl"))
        .ok_or_else(|| format!("OBJ 材质库引用无效：{raw}"))
}
/// 与 NeoForge 的 dirty-material 规则一致：只转换资源名，不打开原作者的路径。
pub(super) fn texture(raw: &str) -> Option<String> {
    if let Some(without_suffix) = raw.strip_suffix(".png") {
        let normalized = without_suffix.replace('\\', "/");
        let converted = if let Some((_, tail)) = normalized.rsplit_once("/assets/") {
            tail.split_once("/textures/")
                .map(|(namespace, path)| (Some(namespace), path))
        } else {
            Some((None, normalized.as_str()))
        };
        if let Some((namespace, path)) = converted {
            let supported = |value: &str| {
                !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b"_-/".contains(&b))
            };
            if supported(path) && namespace.is_none_or(supported) {
                return normalize_texture(&format!("{}:{path}", namespace.unwrap_or("minecraft")));
            }
        }
    }
    normalize_texture(raw)
}
pub(super) fn cull(xyz: [[f32; 3]; 3], normal: [f32; 3]) -> Option<Face> {
    for face in [
        Face::NegX,
        Face::PosX,
        Face::NegZ,
        Face::PosZ,
        Face::Bottom,
        Face::Top,
    ] {
        let direction = face.normal();
        let direction = [direction.0, direction.1, direction.2];
        let axis = direction.iter().position(|v| *v != 0).unwrap();
        let sign = direction[axis] as f32;
        if normal[axis] * sign > 0.0 && xyz.iter().all(|p| (p[axis] - sign * 0.5).abs() < 1e-5) {
            return Some(face);
        }
    }
    None
}
impl ModelBank {
    pub(super) fn bake_obj(&self, spec: MeshSpec<'_>) -> Result<Vec<ObjSelection>, String> {
        let raw = spec
            .document
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or("OBJ 必须声明 model 资源 ID")?;
        let source = resource_ids::qualified(raw, "minecraft")
            .filter(|id| obj_source_suffix(id).is_some())
            .ok_or("OBJ model 资源 ID 无效")?;
        let mesh = self
            .obj_sources
            .get(&source)
            .ok_or_else(|| format!("OBJ 资源不存在：{source}"))?;
        let flag = |name: &str, default| {
            spec.document
                .get(name)
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| format!("OBJ {name} 必须为布尔值"))
                })
                .unwrap_or(Ok(default))
        };
        let flip = flag("flip_v", false)?;
        let automatic_culling = flag("automatic_culling", true)?;
        flag("shade_quads", true)?;
        flag("emissive_ambient", true)?;
        let override_library = spec
            .document
            .get("mtl_override")
            .map(|v| library(&source, v.as_str().ok_or("mtl_override 必须为资源字符串")?))
            .transpose()?;
        let libraries = mesh
            .libraries
            .iter()
            .map(|raw| library(&source, raw))
            .collect::<Result<Vec<_>, _>>()?;
        let mut result = Vec::new();
        for (triangle, attributes) in mesh.triangles.iter().zip(&mesh.attributes) {
            if attributes
                .components
                .iter()
                .any(|name| !spec.visibility.get(name).copied().unwrap_or(true))
            {
                continue;
            }
            // 与游戏相同：未绑定材质的面不生成烘焙面。
            if triangle.material.is_empty() {
                continue;
            }
            let id = override_library
                .as_ref()
                .or_else(|| attributes.library.and_then(|i| libraries.get(i)))
                .ok_or_else(|| format!("OBJ 材质 {} 没有材质库：{source}", triangle.material))?;
            let materials = self
                .obj_libraries
                .get(id)
                .ok_or_else(|| format!("MTL 资源不存在：{id}"))?;
            let material = materials
                .get(&triangle.material)
                .ok_or_else(|| format!("MTL {id} 缺少材质 {}", triangle.material))?;
            let mut texture = material.map_kd.as_deref().unwrap_or("mcedit2:missingno");
            let mut seen = std::collections::BTreeSet::new();
            while let Some(alias) = texture.strip_prefix('#') {
                if !seen.insert(alias) {
                    return Err(format!("OBJ 纹理别名循环：{alias}"));
                }
                texture = spec
                    .textures
                    .get(alias)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("OBJ 纹理别名未定义：#{alias}"))?;
            }
            let texture = self::texture(texture)
                .ok_or_else(|| format!("OBJ 材质纹理资源 ID 无效：{texture}"))?;
            let mut xyz = triangle.xyz.map(|p| spec.root.point(p));
            let mut uv = triangle
                .uv
                .map(|p| [p[0], if flip { 1.0 - p[1] } else { p[1] }]);
            let mut colors = attributes.colors;
            let normal = spec.root.normal(attributes.normals[0]);
            if spec.root.determinant() < 0.0 {
                xyz.swap(1, 2);
                uv.swap(1, 2);
                colors.swap(1, 2);
            }
            if xyz
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1.0e7)
            {
                return Err(format!("OBJ 根变换后坐标无效：{source}"));
            }
            let mut tint = material.diffuse;
            tint[3] = material.diffuse_alpha;
            result.push(ObjSelection {
                material_flags: 0,
                xyz,
                uv,
                texture,
                tint,
                colors,
                alpha_mode: None,
                alpha_cutoff: 0.5,
                double_sided: false,
                cull: automatic_culling.then(|| cull(xyz, normal)).flatten(),
                render_type: spec.render_type,
                tint_index: u32::try_from(material.tint_index).ok(),
            });
        }
        Ok(result)
    }
    pub(crate) fn obj_texture_dependencies(&self) -> std::collections::BTreeSet<String> {
        // JSON 纹理表已涵盖别名绑定；这里只补充 MTL 的直接资源引用。
        self.obj_libraries
            .values()
            .flat_map(|materials| materials.values())
            .filter_map(|material| material.map_kd.as_deref())
            .filter(|raw| !raw.starts_with('#'))
            .filter_map(texture)
            .collect()
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored_filesystem_references_become_resource_ids_without_filesystem_access() {
        assert_eq!(
            texture(r"C:\Original Folder\assets\demo\textures\item\panel.png"),
            Some("demo:item/panel".into())
        );
        assert_eq!(
            texture("/author/work/assets/demo/textures/block/casing.png"),
            Some("demo:casing".into())
        );
        assert_eq!(texture("block/stone.png"), Some("minecraft:stone".into()));
        assert_eq!(
            texture("demo:item/panel.png"),
            Some("demo:item/panel.png".into()),
            "显式资源名不套用文件系统路径规则"
        );
        assert!(texture("../private.png").is_none());
        assert!(texture("https://example.invalid/image.png").is_none());
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Models {
            authored_filesystem_references_become_resource_ids_without_filesystem_access();
        }
    }
}
