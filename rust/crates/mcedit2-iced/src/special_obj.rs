//! Wavefront OBJ 数据解析；材质库仅作为资源 ID 返回，不访问文件或执行模组代码。
#[derive(Clone, Debug)]
pub(crate) struct Triangle {
    pub xyz: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub material: String,
}
#[derive(Clone, Debug)]
pub(crate) struct Attributes {
    pub components: Vec<String>,
    pub library: Option<usize>,
    pub normals: [[f32; 3]; 3],
    pub colors: [[f32; 4]; 3],
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Document {
    pub triangles: std::sync::Arc<Vec<Triangle>>,
    pub attributes: Vec<Attributes>,
    pub libraries: Vec<String>,
}
fn index(raw: &str, len: usize, before: usize) -> Result<usize, String> {
    let n: i64 = raw.parse().map_err(|_| "Invalid OBJ vertex/UV index")?;
    let idx = if n < 0 { before as i64 + n } else { n - 1 };
    if idx < 0 || idx >= len as i64 {
        return Err("OBJ index outside declared data".into());
    }
    Ok(idx as usize)
}
pub(crate) fn parse(bytes: &[u8]) -> Result<Document, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "OBJ must be UTF-8")?;
    let mut vertices = Vec::new();
    let mut coords = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut document = Document::default();
    let mut material = String::new();
    let mut group: Option<String> = None;
    let mut object = None;
    let mut object_above_group = false;
    let mut library = None;
    let mut material_library = None;
    let mut raw_faces = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("v") => {
                let mut p = [0.0; 3];
                for v in &mut p {
                    *v = fields
                        .next()
                        .ok_or("Incomplete OBJ vertex")?
                        .parse::<f32>()
                        .map_err(|_| "Invalid OBJ position")?;
                    if !v.is_finite() {
                        return Err("OBJ position must be finite".into());
                    }
                }
                let w = fields
                    .next()
                    .filter(|s| !s.starts_with('#'))
                    .map(|v| v.parse::<f32>().map_err(|_| "OBJ 齐次坐标无效"))
                    .transpose()?
                    .unwrap_or(1.0);
                if !w.is_finite() || w == 0.0 {
                    return Err("OBJ 齐次坐标不能为零或非有限数".into());
                }
                p = p.map(|v| v / w - 0.5);
                if p.iter().any(|v| !v.is_finite()) {
                    return Err("OBJ 齐次变换后坐标越界".into());
                }
                vertices.push(p);
            }
            Some("vt") => {
                let mut uv = [0.0; 2];
                for v in &mut uv {
                    *v = fields
                        .next()
                        .ok_or("Incomplete OBJ UV")?
                        .parse::<f32>()
                        .map_err(|_| "Invalid OBJ UV")?;
                    if !v.is_finite() {
                        return Err("OBJ UV must be finite".into());
                    }
                }
                coords.push(uv);
            }
            Some(tag @ ("vn" | "vc")) => {
                let is_color = tag == "vc";
                let mut value = if is_color {
                    [0.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0; 4]
                };
                for item in value.iter_mut().take(if is_color { 4 } else { 3 }) {
                    let Some(token) = fields.next().filter(|s| !s.starts_with('#')) else {
                        break;
                    };
                    *item = token.parse::<f32>().map_err(|_| "OBJ 法线或颜色无效")?;
                    if !item.is_finite() {
                        return Err("OBJ 法线或颜色必须为有限数".into());
                    }
                }
                if is_color {
                    colors.push(value);
                } else {
                    normals.push([value[0], value[1], value[2]]);
                }
            }
            Some("usemtl") => {
                let name = fields
                    .take_while(|s| !s.starts_with('#'))
                    .collect::<Vec<_>>()
                    .join(" ");
                if name.is_empty() {
                    return Err("Invalid OBJ material identifier".into());
                }
                material = name;
                material_library = library;
            }
            Some("f") => {
                let entries: Vec<_> = fields.take_while(|s| !s.starts_with('#')).collect();
                if entries.len() < 3 {
                    return Err("OBJ 面至少需要三个顶点".into());
                }
                raw_faces.push((
                    entries,
                    [vertices.len(), coords.len(), normals.len(), colors.len()],
                    group
                        .iter()
                        .chain(object.iter())
                        .cloned()
                        .collect::<Vec<_>>(),
                    material_library,
                    material.clone(),
                    line_number,
                ));
            }
            Some("mtllib") => {
                let name = fields.next().ok_or("OBJ 材质库引用为空")?;
                document.libraries.push(name.into());
                library = Some(document.libraries.len() - 1);
            }
            Some(tag @ ("o" | "g")) => {
                let name = fields.next().ok_or("OBJ 组件名为空")?;
                if tag == "o" && (object_above_group || group.is_none()) {
                    object_above_group = true;
                    group = Some(name.into());
                    object = None;
                } else if object_above_group || tag == "o" {
                    object = Some(format!("{}/{name}", group.as_deref().unwrap_or("")));
                } else {
                    group = Some(name.into());
                    object = None;
                }
            }
            Some("#") | Some("s") | None => {}
            Some(_) => {}
        }
    }
    for (entries, counts, components, library, material, line_number) in raw_faces {
        let mut face = Vec::new();
        for (corner, entry) in entries.iter().enumerate() {
            let values = entry.split('/').collect::<Vec<_>>();
            if values.len() > 4 {
                return Err(format!("OBJ 第 {} 行顶点索引字段过多", line_number + 1));
            }
            let p = index(values[0], vertices.len(), counts[0])?;
            let read = |part: usize, len: usize| {
                values
                    .get(part)
                    .filter(|v| !v.is_empty())
                    .map(|v| index(v, len, counts[part]))
                    .transpose()
            };
            let uv = read(1, coords.len())?
                .map(|i| coords[i])
                .unwrap_or([[0., 0.], [0., 1.], [1., 1.], [1., 0.]][corner.min(3)]);
            let normal = read(2, normals.len())?.map(|i| normals[i]);
            let color = read(3, colors.len())?.map(|i| colors[i]).unwrap_or([1.; 4]);
            face.push((vertices[p], uv, normal, color));
        }
        // NeoForge 将多边形烘焙成一个四角面，采用前四个顶点。
        face.truncate(4);
        let a: [f32; 3] = std::array::from_fn(|i| face[1].0[i] - face[0].0[i]);
        let b: [f32; 3] = std::array::from_fn(|i| face[2].0[i] - face[0].0[i]);
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let length = n.iter().map(|v| v * v).sum::<f32>().sqrt();
        let n = if length > 0. {
            n.map(|v| v / length)
        } else {
            [0.; 3]
        };
        let groups = if face.len() == 3 {
            vec![[0, 1, 2]]
        } else {
            vec![[0, 1, 2], [0, 2, 3]]
        };
        let recalculate = face.iter().any(|point| point.2.is_none());
        for ids in groups {
            std::sync::Arc::make_mut(&mut document.triangles).push(Triangle {
                xyz: ids.map(|i| face[i].0),
                uv: ids.map(|i| face[i].1),
                material: material.clone(),
            });
            document.attributes.push(Attributes {
                components: components.clone(),
                library,
                normals: ids.map(|i| if recalculate { n } else { face[i].2.unwrap() }),
                colors: ids.map(|i| face[i].3),
            });
        }
    }
    Ok(document)
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forward_uv_indices_negative_vertices_groups_normals_and_colors_are_preserved() {
        let source=b"mtllib material.mtl\no Visible\ng Surface\nusemtl Painted Body\nv 0 0 0\nv 2 0 0 2\nv 0 1 0\nf -3/1/1/1 -2/2/1/2 -1/3/1/3\nvt 0 0\nvt 1 0\nvt 0 1\nvn 0 0 1\nvc 1 0 0 1\nvc 0 1 0 0.5\nvc 0 0 1 0.25\n";
        let mesh = parse(source).unwrap();
        assert_eq!(mesh.triangles.len(), 1);
        assert_eq!(mesh.triangles[0].xyz[1], [0.5, -0.5, -0.5]);
        assert_eq!(
            mesh.attributes[0].components,
            vec!["Visible", "Visible/Surface"]
        );
        assert_eq!(mesh.attributes[0].colors[2], [0., 0., 1., 0.25]);
        assert_eq!(mesh.attributes[0].normals, [[0., 0., 1.]; 3]);
        assert_eq!(mesh.attributes[0].library, Some(0));
        assert_eq!(mesh.libraries, vec!["material.mtl"]);
        assert!(parse(b"v 0 0 0\nf 1 2 3\n").is_err());
    }

    fn static_material_quad_triangulates_with_bottom_origin_uv() {
        let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl casing\nf 1/1 2/2 3/3 4/4\n";
        let triangles = parse(obj).unwrap().triangles;
        assert_eq!(triangles.len(), 2);
        assert_eq!(triangles[0].material, "casing");
        assert_eq!(triangles[0].xyz[0], [-0.5, -0.5, -0.5]);
        assert_eq!(triangles[0].uv[0], [0.0, 0.0]);
        assert_eq!(triangles[1].xyz[2], [-0.5, 0.5, -0.5]);
    }

    fn obj_parser_rejects_bad_indices_and_nan() {
        assert!(parse(b"v 0 0 0\nf 2 1 1\n").is_err());
        assert!(parse(b"v NaN 0 0\nvt 0 0\nf 1/1 1/1 1/1\n").is_err());
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Models {
            forward_uv_indices_negative_vertices_groups_normals_and_colors_are_preserved();
            static_material_quad_triangulates_with_bottom_origin_uv();
            obj_parser_rejects_bad_indices_and_nan();
        }
    }
}
