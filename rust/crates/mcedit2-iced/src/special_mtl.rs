//! Strict, read-only subset of Wavefront MTL material metadata. The renderer
//! never opens the mtllib path: materials must be present as pack members.
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub(crate) struct Material {
    pub diffuse: [f32; 4],
    pub map_kd: Option<String>,
    pub map_d: Option<String>,
    pub ambient: [f32; 3],
    pub specular: [f32; 3],
    pub emissive: [f32; 3],
    pub shininess: f32,
    pub refraction: f32,
    pub roughness: Option<f32>,
    pub metallic: Option<f32>,
    pub illum: u8,
    /// NeoForge 将 Kd 的第四分量用于顶点 Alpha，不将 d/Tr 当作该分量。
    pub diffuse_alpha: f32,
    pub tint_index: i32,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            diffuse: [1.; 4],
            map_kd: None,
            map_d: None,
            ambient: [0.; 3],
            specular: [0.; 3],
            emissive: [0.; 3],
            shininess: 0.,
            refraction: 1.,
            roughness: None,
            metallic: None,
            illum: 2,
            diffuse_alpha: 1.,
            tint_index: -1,
        }
    }
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
        && !s.contains("..")
        && !s.contains("//")
        && !s.starts_with('/')
}
fn color(s: &str) -> Result<f32, String> {
    let v = s.parse::<f32>().map_err(|_| "Invalid MTL scalar")?;
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err("MTL scalar outside 0..1".into())
    }
}
pub(crate) fn parse(bytes: &[u8]) -> Result<BTreeMap<String, Material>, String> {
    parse_profile(bytes, false)
}
pub(crate) fn parse_java(bytes: &[u8]) -> Result<BTreeMap<String, Material>, String> {
    parse_profile(bytes, true)
}
fn parse_profile(bytes: &[u8], java: bool) -> Result<BTreeMap<String, Material>, String> {
    let content = std::str::from_utf8(bytes).map_err(|_| "MTL is not UTF-8")?;
    let mut out = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("newmtl") => {
                let name = parts.collect::<Vec<_>>().join(" ");
                if name.is_empty() {
                    return Err("MTL 材质名为空".into());
                }
                current = Some(name.clone());
                out.insert(name, Material::default());
            }
            Some(
                tag @ ("Kd" | "Ka" | "Ks" | "Ke" | "Ns" | "Ni" | "Pr" | "Pm" | "illum" | "d" | "Tr"
                | "map_Kd" | "map_d" | "forge_TintIndex" | "neoforge_TintIndex"),
            ) => {
                let name = current.as_ref().ok_or("MTL 属性出现在 newmtl 声明之前")?;
                let mat = out.get_mut(name).ok_or("Lost material")?;
                if java && matches!(tag, "Ke" | "Ni" | "Pr" | "Pm" | "illum" | "map_d") {
                    continue;
                }
                let channel = |raw: &str| -> Result<f32, String> {
                    if java {
                        raw.parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .ok_or_else(|| "MTL 通道必须为有限数".into())
                    } else {
                        color(raw)
                    }
                };
                match tag {
                    "Kd" => {
                        for k in 0..3 {
                            mat.diffuse[k] = channel(parts.next().ok_or("Missing RGB channel")?)?;
                        }
                        if let Some(alpha) = parts.next().filter(|s| !s.starts_with('#')) {
                            mat.diffuse_alpha = channel(alpha)?;
                            mat.diffuse[3] = mat.diffuse_alpha;
                        }
                    }
                    "Ka" | "Ks" | "Ke" => {
                        let out = match tag {
                            "Ka" => &mut mat.ambient,
                            "Ks" => &mut mat.specular,
                            _ => &mut mat.emissive,
                        };
                        for value in out {
                            *value = channel(parts.next().ok_or("Missing MTL material color")?)?;
                        }
                    }
                    "Ns" | "Ni" | "Pr" | "Pm" => {
                        let v = parts
                            .next()
                            .ok_or("Missing MTL material scalar")?
                            .parse::<f32>()
                            .map_err(|_| "Invalid MTL scalar")?;
                        let (lo, hi) = match tag {
                            "Ns" => (0., 1000.),
                            "Ni" => (1., 3.),
                            _ => (0., 1.),
                        };
                        if !v.is_finite() || !(lo..=hi).contains(&v) {
                            return Err("MTL material scalar out of range".into());
                        }
                        match tag {
                            "Ns" => mat.shininess = v,
                            "Ni" => mat.refraction = v,
                            "Pr" => mat.roughness = Some(v),
                            _ => mat.metallic = Some(v),
                        }
                    }
                    "illum" => {
                        let model = parts
                            .next()
                            .ok_or("Missing illum model")?
                            .parse::<u8>()
                            .map_err(|_| "Invalid illumination model")?;
                        if model > 10 {
                            return Err("Unsupported MTL illum index".into());
                        }
                        mat.illum = model;
                    }
                    "d" => mat.diffuse[3] = color(parts.next().ok_or("Missing opacity")?)?,
                    "Tr" => {
                        mat.diffuse[3] = 1.0 - color(parts.next().ok_or("Missing transparency")?)?
                    }
                    "map_Kd" | "map_d" => {
                        let token = parts.last().ok_or("Missing texture")?;
                        if !(java
                            || identifier(token)
                            || token.strip_prefix('#').is_some_and(identifier))
                        {
                            return Err("Invalid relative material texture ID".into());
                        }
                        if tag == "map_Kd" {
                            mat.map_kd = Some(token.to_owned())
                        } else {
                            mat.map_d = Some(token.to_owned())
                        }
                    }
                    "forge_TintIndex" | "neoforge_TintIndex" => {
                        mat.tint_index = parts
                            .next()
                            .ok_or("MTL tint index 缺失")?
                            .parse()
                            .map_err(|_| "MTL tint index 无效")?;
                        if mat.tint_index < -1 {
                            return Err("MTL tint index 不能小于 -1".into());
                        }
                    }
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_mtl_keeps_raw_resource_references_and_ignores_unused_pbr_fields() {
        let source=b"newmtl Painted Body\nKd 1 1 1 0.6\nd 0.1\nKe 3 3 3\nNi unused\nmap_Kd C:\\Original Folder\\assets\\demo\\textures\\item\\panel.png\nneoforge_TintIndex 2\n";
        let materials = parse_java(source).unwrap();
        assert_eq!(materials["Painted Body"].diffuse_alpha, 0.6);
        assert_eq!(materials["Painted Body"].tint_index, 2);
        assert_eq!(
            materials["Painted Body"].map_kd.as_deref(),
            Some("Folder\\assets\\demo\\textures\\item\\panel.png")
        );
        assert!(parse_java(b"Kd 1 1 1\n").is_err());
    }

    #[test]
    fn reads_static_mtl_diffuse_alpha_and_texture() {
        let m=parse(b"newmtl frame\nKd 0.25 0.5 1\nd 0.6\nmap_Kd example:item/casing\nmap_d example:item/casing_alpha\n").unwrap();
        assert_eq!(m["frame"].diffuse, [0.25, 0.5, 1., 0.6]);
        assert_eq!(m["frame"].map_kd.as_deref(), Some("example:item/casing"));
    }

    #[test]
    fn parses_static_specular_emission_and_pbr_extension_values() {
        let m=parse(b"newmtl chrome\nKs 0.5 0.25 1\nKe 0.1 0.2 0.3\nNs 100\nNi 1.4\nPm 0.8\nPr 0.35\nillum 2\n").unwrap();
        let c = &m["chrome"];
        assert_eq!(c.specular, [0.5, 0.25, 1.]);
        assert_eq!(c.emissive, [0.1, 0.2, 0.3]);
        assert_eq!(c.shininess, 100.);
        assert_eq!(c.refraction, 1.4);
        assert_eq!(c.metallic, Some(0.8));
        assert_eq!(c.roughness, Some(0.35));
        assert!(parse(b"newmtl bad\nNs NaN\n").is_err());
    }

    #[test]
    fn refuses_path_traversal_and_invalid_opacity() {
        assert!(parse(b"newmtl test\nmap_Kd ../private.png\n").is_err());
        assert!(parse(b"newmtl test\nd -1\n").is_err());
    }
}
