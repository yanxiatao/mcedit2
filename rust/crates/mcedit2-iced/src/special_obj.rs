//! Bounded static Wavefront OBJ decoder. Does not execute Mod code, import
//! external material libraries, or follow file references from untrusted packs.
#[derive(Clone, Debug)]
pub(crate) struct Triangle {
    pub xyz: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub material: String,
}
const MAX_VERTICES: usize = 16_384;
const MAX_FACES: usize = 8_192;
const MAX_BYTES: usize = 2 * 1024 * 1024;
fn index(raw: &str, len: usize) -> Result<usize, String> {
    let n: i64 = raw.parse().map_err(|_| "Invalid OBJ vertex/UV index")?;
    let idx = if n < 0 { len as i64 + n } else { n - 1 };
    if idx < 0 || idx >= len as i64 {
        return Err("OBJ index outside declared data".into());
    }
    Ok(idx as usize)
}
pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<Triangle>, String> {
    if bytes.len() > MAX_BYTES { return Err("OBJ exceeds 2 MiB budget".into()); }
    let text = std::str::from_utf8(bytes).map_err(|_| "OBJ must be UTF-8")?;
    let mut vertices = Vec::new();
    let mut coords = Vec::new();
    let mut triangles = Vec::new();
    let mut material = String::new();
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("v") => {
                if vertices.len() >= MAX_VERTICES {
                    return Err("OBJ vertex count limit".into());
                }
                let mut p = [0.0; 3];
                for v in &mut p {
                    *v = fields.next().ok_or("Incomplete OBJ vertex")?
                        .parse::<f32>().map_err(|_| "Invalid OBJ position")?;
                    if !v.is_finite() || !(-2.0..=3.0).contains(v) {
                        return Err("OBJ position outside guarded block range".into());
                    }
                    *v -= 0.5;
                }
                vertices.push(p);
            }
            Some("vt") => {
                if coords.len() >= MAX_VERTICES {
                    return Err("OBJ UV count limit".into());
                }
                let mut uv = [0.0; 2];
                for v in &mut uv {
                    *v = fields.next().ok_or("Incomplete OBJ UV")?
                        .parse::<f32>().map_err(|_| "Invalid OBJ UV")?;
                    if !v.is_finite() || !(-16.0..=16.0).contains(v) {
                        return Err("OBJ UV exceeds bounded repeat range".into());
                    }
                }
                coords.push([uv[0], 1.0 - uv[1]]);
            }
            Some("usemtl") => {
                let name = fields.next().ok_or("Empty OBJ material")?;
                if name.len() > 160 ||
                    !name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c)) {
                    return Err("Invalid OBJ material identifier".into());
                }
                material = name.to_string();
            }
            Some("f") => {
                let entries: Vec<_> = fields.collect();
                if !(3..=4).contains(&entries.len()) {
                    return Err("Only triangulated/quad OBJ faces supported".into());
                }
                let mut face = Vec::new();
                for entry in entries {
                    let mut pair = entry.split('/');
                    let p = index(pair.next().unwrap_or(""), vertices.len())?;
                    let u = index(pair.next().ok_or("Missing OBJ face UV")?, coords.len())?;
                    face.push((vertices[p], coords[u]));
                }
                let groups = if face.len() == 3 {
                    vec![[0, 1, 2]]
                } else {
                    vec![[0, 1, 2], [0, 2, 3]]
                };
                for ids in groups {
                    if triangles.len() >= MAX_FACES {
                        return Err("OBJ triangle count limit".into());
                    }
                    triangles.push(Triangle {
                        xyz: ids.map(|i| face[i].0),
                        uv: ids.map(|i| face[i].1),
                        material: material.clone(),
                    });
                }
            }
            Some("mtllib") => { /* No external resources are followed. */ }
            Some("#") | Some("o") | Some("g") | Some("vn") | Some("s") | None => {}
            Some(_) => {}
        }
    }
    if triangles.is_empty() { return Err("OBJ contains no static triangles".into()); }
    Ok(triangles)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_material_quad_triangulates_with_bottom_origin_uv() {
        let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl casing\nf 1/1 2/2 3/3 4/4\n";
        let triangles = parse(obj).unwrap();
        assert_eq!(triangles.len(), 2);
        assert_eq!(triangles[0].material, "casing");
        assert_eq!(triangles[0].xyz[0], [-0.5, -0.5, -0.5]);
        assert_eq!(triangles[0].uv[0], [0.0, 1.0]);
        assert_eq!(triangles[1].xyz[2], [-0.5, 0.5, -0.5]);
    }
    #[test]
    fn obj_parser_rejects_bad_indices_nan_and_traversal() {
        assert!(parse(b"v 0 0 0\nf 1 1 1\n").is_err());
        assert!(parse(b"v NaN 0 0\nvt 0 0\nf 1/1 1/1 1/1\n").is_err());
        assert!(parse(b"v 0 0 0\nvt 0 0\nusemtl ../escape\nf 1/1 1/1 1/1\n").is_err());
    }
}
