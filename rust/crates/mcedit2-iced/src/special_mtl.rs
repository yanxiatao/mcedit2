//! Strict, read-only subset of Wavefront MTL material metadata. The renderer
//! never opens the mtllib path: materials must be present as pack members.
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub(crate) struct Material {
    pub diffuse: [f32; 4],
    pub map_kd: Option<String>,
    pub map_d: Option<String>,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            diffuse: [1.; 4],
            map_kd: None,
            map_d: None,
        }
    }
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
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
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("MTL exceeds file limit".into());
    }
    let content = std::str::from_utf8(bytes).map_err(|_| "MTL is not UTF-8")?;
    let mut out = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("newmtl") => {
                let name = parts.next().ok_or("Missing MTL material name")?;
                if !identifier(name) || out.len() >= 2048 {
                    return Err("Invalid MTL name or too many materials".into());
                }
                current = Some(name.to_owned());
                out.insert(name.to_owned(), Material::default());
            }
            Some(tag @ ("Kd" | "d" | "Tr" | "map_Kd" | "map_d")) => {
                let Some(name) = current.as_ref() else {
                    continue;
                };
                let mat = out.get_mut(name).ok_or("Lost material")?;
                match tag {
                    "Kd" => {
                        for k in 0..3 {
                            mat.diffuse[k] = color(parts.next().ok_or("Missing RGB channel")?)?;
                        }
                    }
                    "d" => mat.diffuse[3] = color(parts.next().ok_or("Missing opacity")?)?,
                    "Tr" => {
                        mat.diffuse[3] = 1.0 - color(parts.next().ok_or("Missing transparency")?)?
                    }
                    "map_Kd" | "map_d" => {
                        let token = parts.next().ok_or("Missing texture")?;
                        if !identifier(token) {
                            return Err("Invalid relative material texture ID".into());
                        }
                        if tag == "map_Kd" {
                            mat.map_kd = Some(token.to_owned())
                        } else {
                            mat.map_d = Some(token.to_owned())
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
    fn reads_static_mtl_diffuse_alpha_and_texture() {
        let m=parse(b"newmtl frame\nKd 0.25 0.5 1\nd 0.6\nmap_Kd example:item/casing\nmap_d example:item/casing_alpha\n").unwrap();
        assert_eq!(m["frame"].diffuse, [0.25, 0.5, 1., 0.6]);
        assert_eq!(m["frame"].map_kd.as_deref(), Some("example:item/casing"));
    }
    #[test]
    fn refuses_path_traversal_and_invalid_opacity() {
        assert!(parse(b"newmtl test\nmap_Kd ../private.png\n").is_err());
        assert!(parse(b"newmtl test\nd -1\n").is_err());
    }
}
