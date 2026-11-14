//! Guarded static glTF 2.0 / GLB decoder; only embedded buffers. No fetch,
//! extensions, executable scripts, animations, skinned meshes, or URI traversal.
use crate::special_obj::Triangle;
use serde_json::Value;
fn number(v: &Value) -> Result<usize, String> {
    let n = v.as_u64().ok_or("glTF integer required")?;
    usize::try_from(n).map_err(|_| "glTF integer overflow".into())
}
fn field<'a>(v: &'a Value, name: &str) -> Result<&'a Value, String> {
    v.get(name).ok_or_else(|| format!("glTF missing {name}"))
}
fn item(v: &Value, at: usize) -> Result<&Value, String> {
    v.as_array()
        .and_then(|a| a.get(at))
        .ok_or("glTF array index out of range".into())
}
fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut bits: u32 = 0;
    let mut count = 0;
    let mut padded = false;
    for c in input.bytes() {
        if c == b'=' {
            padded = true;
            continue;
        }
        if padded {
            return Err("Invalid trailing glTF base64".into());
        }
        let n: u32 = match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return Err("Invalid glTF base64".into()),
        };
        bits = (bits << 6) | n;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Ok(out)
}
struct Accessor<'a> {
    data: &'a [u8],
    start: usize,
    stride: usize,
    length: usize,
    count: usize,
    kind: usize,
}
fn accessor<'a>(doc: &Value, buffers: &'a [Vec<u8>], at: usize) -> Result<Accessor<'a>, String> {
    let acc = item(field(doc, "accessors")?, at)?;
    if acc.get("sparse").is_some() {
        return Err("Sparse glTF accessors unsupported".into());
    }
    let count = number(field(acc, "count")?)?;
    let kind = number(field(acc, "componentType")?)?;
    let channels = match field(acc, "type")?.as_str() {
        Some("SCALAR") => 1,
        Some("VEC2") => 2,
        Some("VEC3") => 3,
        _ => return Err("Unsupported glTF accessor type".into()),
    };
    let component = match kind {
        5126 | 5125 => 4,
        5123 => 2,
        5121 => 1,
        _ => return Err("Unsupported glTF component type".into()),
    };
    let length = channels * component;
    let view = item(
        field(doc, "bufferViews")?,
        number(field(acc, "bufferView")?)?,
    )?;
    let data = buffers
        .get(number(field(view, "buffer")?)?)
        .ok_or("Invalid glTF buffer index")?;
    let bytes = data.as_slice();
    let view_start = view.get("byteOffset").map(number).transpose()?.unwrap_or(0);
    let start = view_start
        .checked_add(acc.get("byteOffset").map(number).transpose()?.unwrap_or(0))
        .ok_or("glTF buffer offset overflow")?;
    let stride = view
        .get("byteStride")
        .map(number)
        .transpose()?
        .unwrap_or(length);
    if stride < length {
        return Err("Invalid glTF accessor stride/count".into());
    }
    let end = start
        .checked_add(
            count
                .saturating_sub(1)
                .checked_mul(stride)
                .ok_or("glTF stride overflow")?,
        )
        .and_then(|n| n.checked_add(length))
        .ok_or("glTF accessor overflow")?;
    let view_end = view_start
        .checked_add(number(field(view, "byteLength")?)?)
        .ok_or("glTF view overflow")?;
    if end > bytes.len() || end > view_end {
        return Err("glTF accessor outside bufferView".into());
    }
    Ok(Accessor {
        data: bytes,
        start,
        stride,
        length,
        count,
        kind,
    })
}
impl Accessor<'_> {
    fn point(&self, i: usize, channels: usize) -> Result<Vec<f32>, String> {
        if self.kind != 5126 || self.length != channels * 4 || i >= self.count {
            return Err("Invalid glTF float accessor".into());
        }
        let p = self.start + i * self.stride;
        (0..channels)
            .map(|k| {
                let n = f32::from_le_bytes(self.data[p + 4 * k..p + 4 * k + 4].try_into().unwrap());
                if n.is_finite() {
                    Ok(n)
                } else {
                    Err("glTF coordinate must be finite".into())
                }
            })
            .collect()
    }
    fn index(&self, i: usize) -> Result<usize, String> {
        if self.length > 4 || i >= self.count {
            return Err("Invalid glTF index accessor".into());
        }
        let p = self.start + i * self.stride;
        Ok(match self.kind {
            5121 => self.data[p] as usize,
            5123 => u16::from_le_bytes(self.data[p..p + 2].try_into().unwrap()) as usize,
            5125 => u32::from_le_bytes(self.data[p..p + 4].try_into().unwrap()) as usize,
            _ => return Err("glTF indices must be unsigned".into()),
        })
    }
}
fn f64_val(v: Option<&Value>, default: f64) -> Result<f32, String> {
    let n = v
        .map(|v| v.as_f64().ok_or("Invalid glTF transform"))
        .transpose()?
        .unwrap_or(default);
    if !(n as f32).is_finite() {
        return Err("Invalid glTF transform".into());
    }
    Ok(n as f32)
}
type Matrix = [f32; 16];
fn identity() -> Matrix {
    [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]
}
fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|k| {
        let (r, c) = (k % 4, k / 4);
        (0..4).map(|i| a[i * 4 + r] * b[c * 4 + i]).sum()
    })
}
fn transform(node: &Value) -> Result<Matrix, String> {
    if let Some(matrix) = node.get("matrix") {
        if node.get("translation").is_some()
            || node.get("rotation").is_some()
            || node.get("scale").is_some()
        {
            return Err("glTF node cannot mix matrix and TRS".into());
        }
        let arr = matrix
            .as_array()
            .filter(|a| a.len() == 16)
            .ok_or("Invalid glTF matrix")?;
        let mut out = [0.; 16];
        for (i, v) in arr.iter().enumerate() {
            out[i] = f64_val(Some(v), 0.)?;
        }
        return Ok(out);
    }
    let scale = node
        .get("scale")
        .map(|v| v.as_array().ok_or("Invalid glTF scale"))
        .transpose()?;
    let rot = node
        .get("rotation")
        .map(|v| v.as_array().ok_or("Invalid glTF rotation"))
        .transpose()?;
    let trans = node
        .get("translation")
        .map(|v| v.as_array().ok_or("Invalid glTF translation"))
        .transpose()?;
    if scale.is_some_and(|a| a.len() != 3)
        || rot.is_some_and(|a| a.len() != 4)
        || trans.is_some_and(|a| a.len() != 3)
    {
        return Err("Invalid glTF TRS vector lengths".into());
    }
    let s = [
        f64_val(scale.and_then(|v| v.first()), 1.)?,
        f64_val(scale.and_then(|v| v.get(1)), 1.)?,
        f64_val(scale.and_then(|v| v.get(2)), 1.)?,
    ];
    let (x, y, z, w) = (
        f64_val(rot.and_then(|v| v.first()), 0.)?,
        f64_val(rot.and_then(|v| v.get(1)), 0.)?,
        f64_val(rot.and_then(|v| v.get(2)), 0.)?,
        f64_val(rot.and_then(|v| v.get(3)), 1.)?,
    );
    let n = x * x + y * y + z * z + w * w;
    if !n.is_finite() || n < 1e-12 {
        return Err("Degenerate glTF quaternion".into());
    }
    let q = n.sqrt().recip();
    let (x, y, z, w) = (x * q, y * q, z * q, w * q);
    let t = [
        f64_val(trans.and_then(|v| v.first()), 0.)?,
        f64_val(trans.and_then(|v| v.get(1)), 0.)?,
        f64_val(trans.and_then(|v| v.get(2)), 0.)?,
    ];
    Ok([
        (1. - 2. * (y * y + z * z)) * s[0],
        2. * (x * y + z * w) * s[0],
        2. * (x * z - y * w) * s[0],
        0.,
        2. * (x * y - z * w) * s[1],
        (1. - 2. * (x * x + z * z)) * s[1],
        2. * (y * z + x * w) * s[1],
        0.,
        2. * (x * z + y * w) * s[2],
        2. * (y * z - x * w) * s[2],
        (1. - 2. * (x * x + y * y)) * s[2],
        0.,
        t[0],
        t[1],
        t[2],
        1.,
    ])
}
fn apply_point(m: Matrix, p: [f32; 3]) -> Result<[f32; 3], String> {
    let v = std::array::from_fn::<_, 3, _>(|i| {
        m[12 + i] + (0..3).map(|j| m[4 * j + i] * p[j]).sum::<f32>()
    });
    if v.iter().all(|x| x.is_finite()) {
        Ok(v)
    } else {
        Err("glTF transformed point must be finite".into())
    }
}
/// Scene traversal is limited to selected roots; transforms compose from
/// parent to child. Repeated instances are legal, ancestry cycles are not.
fn instances(doc: &Value, mesh_count: usize) -> Result<Vec<(usize, Matrix)>, String> {
    let Some(value) = doc.get("nodes") else {
        return Ok((0..mesh_count).map(|i| (i, identity())).collect());
    };
    let nodes = value.as_array().ok_or("glTF nodes must be an array")?;
    let children = |node: &Value| -> Result<Vec<usize>, String> {
        node.get("children")
            .map(|value| {
                value
                    .as_array()
                    .ok_or("glTF children must be an array")?
                    .iter()
                    .map(number)
                    .collect()
            })
            .transpose()
            .map(|v| v.unwrap_or_default())
    };
    let roots = if let Some(value) = doc.get("scenes") {
        let scenes = value.as_array().ok_or("glTF scenes must be an array")?;
        let id = doc.get("scene").map(number).transpose()?.unwrap_or(0);
        let scene = scenes.get(id).ok_or("Invalid scene index")?;
        scene
            .get("nodes")
            .map(|v| {
                v.as_array()
                    .ok_or("Scene nodes must be an array")?
                    .iter()
                    .map(number)
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default()
    } else {
        let mut child = vec![false; nodes.len()];
        for node in nodes {
            for id in children(node)? {
                *child.get_mut(id).ok_or("Invalid child")? = true;
            }
        }
        child
            .iter()
            .enumerate()
            .filter_map(|(i, v)| (!v).then_some(i))
            .collect::<Vec<_>>()
    };
    let mut out = Vec::new();
    let mut active = vec![false; nodes.len()];
    let mut stack = roots
        .into_iter()
        .rev()
        .map(|id| (id, Some(identity())))
        .collect::<Vec<_>>();
    while let Some((id, parent)) = stack.pop() {
        let Some(parent) = parent else {
            active[id] = false;
            continue;
        };
        let node = nodes.get(id).ok_or("Unknown glTF node")?;
        if active[id] {
            return Err("Cycle in glTF node graph".into());
        }
        active[id] = true;
        let current = multiply(parent, transform(node)?);
        if let Some(mesh) = node.get("mesh") {
            out.push((number(mesh)?, current));
        }
        stack.push((id, None));
        for child in children(node)?.into_iter().rev() {
            stack.push((child, Some(current)));
        }
    }
    Ok(out)
}

fn parse_document(doc: Value, buffers: &[Vec<u8>]) -> Result<Vec<Triangle>, String> {
    if field(&doc, "asset")?.get("version").and_then(Value::as_str) != Some("2.0") {
        return Err("glTF 2.0 required".into());
    }
    let meshes = field(&doc, "meshes")?
        .as_array()
        .ok_or("Missing glTF meshes")?;
    let mut out = Vec::new();
    let instances = instances(&doc, meshes.len())?;
    for (id, node) in instances {
        let mesh = meshes.get(id).ok_or("Unknown glTF mesh")?;
        for prim in field(mesh, "primitives")?
            .as_array()
            .ok_or("Invalid glTF primitives")?
        {
            if prim.get("mode").map(number).transpose()?.unwrap_or(4) != 4 {
                return Err("Only glTF TRIANGLES supported".into());
            }
            let attrs = field(prim, "attributes")?;
            let positions = accessor(&doc, buffers, number(field(attrs, "POSITION")?)?)?;
            let uvs = accessor(&doc, buffers, number(field(attrs, "TEXCOORD_0")?)?)?;
            if uvs.count != positions.count {
                return Err("glTF position/UV count mismatch".into());
            }
            let indices = prim
                .get("indices")
                .map(|v| accessor(&doc, buffers, number(v)?))
                .transpose()?;
            let count = indices.as_ref().map_or(positions.count, |i| i.count);
            if count % 3 != 0 {
                return Err("glTF primitive has incomplete triangles".into());
            }
            let material_id = prim.get("material").map(number).transpose()?;
            let material = if let Some(i) = material_id {
                let entry = item(field(&doc, "materials")?, i)?;
                let specified = entry
                    .get("name")
                    .map(|v| v.as_str().ok_or("Invalid glTF material name"))
                    .transpose()?;
                specified
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("__gltf_material_{i}"))
            } else {
                "particle".to_owned()
            };
            for f in (0..count).step_by(3) {
                let mut xyz = [[0.; 3]; 3];
                let mut uv = [[0.; 2]; 3];
                for k in 0..3 {
                    let i = indices.as_ref().map_or(Ok(f + k), |a| a.index(f + k))?;
                    let position = positions.point(i, 3)?;
                    let point = [position[0], position[1], position[2]];
                    let transformed = apply_point(node, point)?;
                    for c in 0..3 {
                        xyz[k][c] = transformed[c] - 0.5;
                    }
                    let coords = uvs.point(i, 2)?;
                    uv[k] = [coords[0], coords[1]];
                }
                out.push(Triangle {
                    xyz,
                    uv,
                    material: material.clone(),
                });
            }
        }
    }
    if out.is_empty() {
        return Err("glTF has no triangles".into());
    }
    Ok(out)
}
fn safe_external_uri(uri: &str) -> bool {
    !uri.is_empty()
        && !uri.starts_with('/')
        && !uri.contains('\\')
        && uri.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
        })
}
/// External buffer reads must be supplied by the already-ingested
/// resource-pack content. URLs, absolute paths and parent traversal are
/// forbidden; no network access or arbitrary filesystem reads.
pub(crate) fn parse_with_resolver(
    bytes: &[u8],
    resolve: &impl Fn(&str) -> Option<Vec<u8>>,
) -> Result<Vec<Triangle>, String> {
    let (doc, embedded) = if bytes.starts_with(b"glTF") {
        if bytes.len() < 20
            || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 2
            || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize != bytes.len()
        {
            return Err("Invalid GLB header".into());
        }
        let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        if len > bytes.len() - 20 || &bytes[16..20] != b"JSON" {
            return Err("Missing GLB JSON chunk".into());
        }
        let doc: Value = serde_json::from_slice(&bytes[20..20 + len]).map_err(|e| e.to_string())?;
        let offset = 20 + len;
        if bytes.len() < offset + 8 || &bytes[offset + 4..offset + 8] != b"BIN\0" {
            return Err("Missing GLB BIN chunk".into());
        }
        let size = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if size > bytes.len() - offset - 8 {
            return Err("Invalid GLB BIN length".into());
        }
        (doc, Some(bytes[offset + 8..offset + 8 + size].to_vec()))
    } else {
        (
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?,
            None,
        )
    };
    let declared = field(&doc, "buffers")?
        .as_array()
        .ok_or("Missing glTF buffers")?;
    if declared.is_empty() {
        return Err("glTF requires buffers".into());
    }
    let mut loaded = Vec::new();
    for (i, buffer) in declared.iter().enumerate() {
        let bytes_needed = number(field(buffer, "byteLength")?)?;
        let bytes = if let Some(uri) = buffer
            .get("uri")
            .map(|v| v.as_str().ok_or("Invalid glTF uri"))
            .transpose()?
        {
            if let Some(encoded) = uri
                .strip_prefix("data:application/octet-stream;base64,")
                .or_else(|| uri.strip_prefix("data:application/gltf-buffer;base64,"))
            {
                decode_base64(encoded)?
            } else if safe_external_uri(uri) {
                resolve(uri).ok_or("Missing pack-local glTF buffer")?
            } else {
                return Err("Unsafe external glTF buffer URI".into());
            }
        } else if i == 0 {
            embedded.clone().ok_or("Missing embedded GLB buffer")?
        } else {
            return Err("Only first GLB buffer may be embedded".into());
        };
        if bytes.len() < bytes_needed {
            return Err("glTF declared buffer length mismatch".into());
        }
        loaded.push(bytes);
    }
    parse_document(doc, &loaded)
}
#[cfg(test)]
pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<Triangle>, String> {
    parse_with_resolver(bytes, &|_| None)
}

pub(crate) use crate::special_gltf_types::StaticPbrMaterial;
/// Read material factors from ordinary glTF JSON or the JSON chunk of GLB.
/// These are data only; metallic-roughness lighting is not inferred from
/// missing scene lights or custom client shaders.
pub(crate) fn material_factors(
    source: &[u8],
) -> Result<std::collections::BTreeMap<String, StaticPbrMaterial>, String> {
    let data = if source.starts_with(b"glTF") {
        if source.len() < 20 {
            return Err("Truncated GLB".into());
        }
        let n = u32::from_le_bytes(source[12..16].try_into().unwrap()) as usize;
        if n > source.len() - 20 || &source[16..20] != b"JSON" {
            return Err("GLB lacks JSON material chunk".into());
        }
        &source[20..20 + n]
    } else {
        source
    };
    let doc: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let mut out = std::collections::BTreeMap::new();
    let Some(materials) = doc
        .get("materials")
        .map(|v| v.as_array().ok_or("Invalid glTF materials"))
        .transpose()?
    else {
        return Ok(out);
    };
    let bounded = |v: Option<&Value>, default: f32| -> Result<f32, String> {
        let n = v
            .map(|v| v.as_f64().ok_or("Invalid glTF PBR factor"))
            .transpose()?
            .map(|n| n as f32)
            .unwrap_or(default);
        if n.is_finite() && (0.0..=1.0).contains(&n) {
            Ok(n)
        } else {
            Err("glTF PBR factor out of range".into())
        }
    };
    for (index, material) in materials.iter().enumerate() {
        let generated = format!("__gltf_material_{index}");
        let name = material
            .get("name")
            .map(|v| v.as_str().ok_or("Invalid glTF material name"))
            .transpose()?
            .unwrap_or(&generated);
        let pbr = material.get("pbrMetallicRoughness").unwrap_or(&Value::Null);
        if !pbr.is_null() && !pbr.is_object() {
            return Err("Invalid glTF PBR material".into());
        }
        let color = pbr
            .get("baseColorFactor")
            .map(|v| v.as_array().ok_or("Invalid glTF baseColorFactor"))
            .transpose()?;
        if color.is_some_and(|v| v.len() != 4) {
            return Err("Invalid glTF RGBA factor".into());
        }
        let base = [
            bounded(color.and_then(|v| v.first()), 1.)?,
            bounded(color.and_then(|v| v.get(1)), 1.)?,
            bounded(color.and_then(|v| v.get(2)), 1.)?,
            bounded(color.and_then(|v| v.get(3)), 1.)?,
        ];
        let em = material
            .get("emissiveFactor")
            .map(|v| v.as_array().ok_or("Invalid glTF emissiveFactor"))
            .transpose()?;
        if em.is_some_and(|v| v.len() != 3) {
            return Err("Invalid emissive color".into());
        }
        let emissive = [
            bounded(em.and_then(|v| v.first()), 0.)?,
            bounded(em.and_then(|v| v.get(1)), 0.)?,
            bounded(em.and_then(|v| v.get(2)), 0.)?,
        ];
        let alpha_mode = material
            .get("alphaMode")
            .map(|v| v.as_str().ok_or("Invalid glTF alphaMode"))
            .transpose()?
            .unwrap_or("OPAQUE");
        if !matches!(alpha_mode, "OPAQUE" | "MASK" | "BLEND") {
            return Err("Invalid glTF alphaMode".into());
        }
        let cutoff = bounded(material.get("alphaCutoff"), 0.5)?;
        let base_color_uri = if let Some(color_texture) = pbr.get("baseColorTexture") {
            if color_texture
                .get("texCoord")
                .is_some_and(|v| v.as_u64() != Some(0))
            {
                return Err("Only TEXCOORD_0 is currently supported".into());
            }
            let textures = field(&doc, "textures")?
                .as_array()
                .ok_or("Missing glTF textures")?;
            let index = number(field(color_texture, "index")?)?;
            let descriptor = textures
                .get(index)
                .ok_or("glTF texture index out of range")?;
            let image_index = number(field(descriptor, "source")?)?;
            let images = field(&doc, "images")?
                .as_array()
                .ok_or("Missing glTF images")?;
            let image = images
                .get(image_index)
                .ok_or("glTF image index out of range")?;
            // Embedded bufferView images are not automatically uploaded yet;
            // preserve geometry and explicit Java-side texture overrides.
            image
                .get("uri")
                .map(|v| v.as_str().ok_or("Invalid glTF image URI"))
                .transpose()?
                .map(str::to_owned)
        } else {
            None
        };
        out.insert(
            name.to_owned(),
            StaticPbrMaterial {
                base_color: base,
                metallic: bounded(pbr.get("metallicFactor"), 1.)?,
                roughness: bounded(pbr.get("roughnessFactor"), 1.)?,
                emissive,
                alpha_mode: alpha_mode.into(),
                alpha_cutoff: cutoff,
                double_sided: material
                    .get("doubleSided")
                    .map(|v| v.as_bool().ok_or("Invalid glTF doubleSided"))
                    .transpose()?
                    .unwrap_or(false),
                base_color_uri,
                base_color_texture: None,
            },
        );
    }
    Ok(out)
}
/// Normalize a URI against its exact resource-pack glTF location. Relative
/// `..` is legal within the same assets/<namespace> tree but can never
/// escape that namespace. No HTTP, OS absolute path, query or percent escapes.
/// Returns the canonical atlas texture ID matching TextureBank::texture_id.
pub(crate) fn pack_image_id(source: &str, uri: &str) -> Option<String> {
    if uri.is_empty()
        || !uri.ends_with(".png")
        || uri.contains('\\')
        || uri.contains(':')
        || uri.contains('?')
        || uri.contains('#')
        || uri.contains('%')
        || uri.starts_with('/')
    {
        return None;
    }
    let source_parts: Vec<_> = source.split('/').collect();
    if source_parts.len() < 5 || source_parts[0] != "assets" || source_parts[2] != "models" {
        return None;
    }
    let namespace = source_parts[1];
    if !crate::resource_ids::namespace(namespace) {
        return None;
    }
    let mut parts: Vec<String> = source_parts[..source_parts.len() - 1]
        .iter()
        .map(|component| (*component).to_owned())
        .collect();
    for component in uri.split('/') {
        match component {
            "" | "." => return None,
            ".." if parts.len() > 2 => {
                parts.pop();
            }
            ".." => return None,
            segment
                if segment.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)
                }) =>
            {
                parts.push(segment.to_owned())
            }
            _ => return None,
        }
    }
    if parts.len() < 4 || parts[0] != "assets" || parts[1] != namespace {
        return None;
    }
    let id = parts[3..].join("/");
    let id = id.strip_suffix(".png")?;
    match parts[2].as_str() {
        "textures" => {
            if !crate::resource_ids::path(id) {
                return None;
            }
            let name = id
                .strip_prefix("block/")
                .or_else(|| id.strip_prefix("blocks/"))
                .unwrap_or(id);
            Some(format!("{namespace}:{name}"))
        }
        "models" => {
            if !crate::resource_ids::path(id) {
                return None;
            }
            Some(format!("{namespace}:__gltf/{id}"))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gltf_base_color_texture_is_resolved_from_nested_images_and_names() {
        for malformed in [
            br#"{"materials":false}"#.as_slice(),
            br#"{"materials":[{"name":false}]}"#.as_slice(),
            br#"{"materials":[{"doubleSided":"true"}]}"#.as_slice(),
            br#"{"materials":[{"emissiveFactor":2}]}"#.as_slice(),
        ] {
            assert!(material_factors(malformed).is_err());
        }
        assert!(transform(&serde_json::json!({"scale":"invalid"})).is_err());
        let data = br#"{"materials":[
           {"pbrMetallicRoughness":{"baseColorTexture":{"index":1}}},
           {"name":"steel","pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}
        ],"textures":[{"source":0},{"source":1}],
        "images":[{"uri":"../../textures/block/steel.png"},{"uri":"panel.png"}]}"#;
        let result = material_factors(data).unwrap();
        assert_eq!(
            result["__gltf_material_0"].base_color_uri.as_deref(),
            Some("panel.png")
        );
        assert_eq!(
            result["steel"].base_color_uri.as_deref(),
            Some("../../textures/block/steel.png")
        );
        assert_eq!(
            pack_image_id("assets/example/models/block/test.gltf", "panel.png"),
            Some("example:__gltf/block/panel".into())
        );
        assert_eq!(
            pack_image_id(
                "assets/example/models/block/test.gltf",
                "../../textures/block/steel.png"
            ),
            Some("example:steel".into())
        );
    }

    fn gltf_image_paths_cannot_escape_namespace_or_load_remote_content() {
        let source = "assets/safe/models/block/machine.gltf";
        for uri in [
            "../../../other/cross.png",
            "../../../../escape.png",
            "https://example.org/img.png",
            "/etc/a.png",
            "foo//x.png",
            "foo\\bad.png",
            "image.jpg",
            "%2e%2e/out.png",
        ] {
            assert!(pack_image_id(source, uri).is_none(), "{uri}");
        }
        assert_eq!(
            pack_image_id(source, "../panel.png"),
            Some("safe:__gltf/panel".into())
        );
    }

    fn hierarchical_scene_uses_parent_child_transforms_and_filters_unreachable() {
        let d = serde_json::json!({"asset":{"version":"2.0"},"meshes":[{}],
            "nodes":[{"translation":[1,0,0],"children":[1]},
                     {"mesh":0,"translation":[0,1,0]},
                     {"mesh":0,"translation":[99,0,0]}],
            "scenes":[{"nodes":[0]}],"scene":0});
        let nodes = instances(&d, 1).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(apply_point(nodes[0].1, [0., 0., 0.]).unwrap(), [1., 1., 0.]);
    }

    fn scene_cycles_and_bad_child_ids_fail_closed() {
        let cyclic = serde_json::json!({"nodes":[{"children":[1]},{"children":[0]}],
            "scenes":[{"nodes":[0]}]});
        assert!(instances(&cyclic, 1).is_err());
        let malformed = serde_json::json!({"nodes":[{"children":[88]}],
            "scenes":[{"nodes":[0]}]});
        assert!(instances(&malformed, 1).is_err());
    }

    fn external_buffer_safety_refuses_remote_and_traversal() {
        for path in [
            "../escape.bin",
            "https://remote/a.bin",
            "/absolute.bin",
            "a//b.bin",
        ] {
            assert!(!safe_external_uri(path));
        }
        assert!(safe_external_uri("meshes/points.bin"));
    }

    fn static_gltf_pbr_color_roughness_metallic_and_alpha_are_validated() {
        let doc = br#"{"asset":{"version":"2.0"},"materials":[{
            "name":"frame","pbrMetallicRoughness":{"baseColorFactor":[0.1,0.2,0.3,0.6],
            "metallicFactor":0.8,"roughnessFactor":0.25},
            "emissiveFactor":[0.02,0.04,0.06],"alphaMode":"BLEND",
            "doubleSided":true}]}"#;
        let mat = material_factors(doc).unwrap();
        assert_eq!(mat["frame"].base_color, [0.1, 0.2, 0.3, 0.6]);
        assert_eq!(mat["frame"].metallic, 0.8);
        assert_eq!(mat["frame"].roughness, 0.25);
        assert_eq!(mat["frame"].emissive, [0.02, 0.04, 0.06]);
        assert_eq!(mat["frame"].alpha_mode, "BLEND");
        assert!(mat["frame"].double_sided);
        assert!(material_factors(
            br#"{"materials":[{"name":"bad","pbrMetallicRoughness":{"metallicFactor":-3}}]}"#
        )
        .is_err());
    }

    fn rejects_remote_external_and_malformed_sources() {
        assert!(parse(
            br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"https://remote","byteLength":4}]}"#
        )
        .is_err());
        assert!(parse(b"glTFbad").is_err());
        assert!(parse(br#"{"asset":{"version":"1.0"}}"#).is_err());
    }

    fn decodes_embedded_triangle() {
        let mut bin = Vec::new();
        for v in [0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1.] {
            bin.extend_from_slice(&v.to_le_bytes());
        }
        let doc = serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":60}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"mode":4}]}]});
        let mut json = serde_json::to_vec(&doc).unwrap();
        while !json.len().is_multiple_of(4) {
            json.push(b' ')
        }
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(12 + 8 + json.len() + 8 + bin.len()).to_le_bytes()[..4]);
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        let out = parse(&glb).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].xyz[0], [-0.5, -0.5, -0.5]);
        assert_eq!(out[0].uv[2], [0., 1.]);
        let mut malformed = doc;
        malformed["accessors"][0]["byteOffset"] = serde_json::json!("wrong");
        assert!(parse_document(malformed, &[bin])
            .unwrap_err()
            .contains("integer"));
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Models {
            gltf_base_color_texture_is_resolved_from_nested_images_and_names();
            gltf_image_paths_cannot_escape_namespace_or_load_remote_content();
            hierarchical_scene_uses_parent_child_transforms_and_filters_unreachable();
            scene_cycles_and_bad_child_ids_fail_closed();
            external_buffer_safety_refuses_remote_and_traversal();
            static_gltf_pbr_color_roughness_metallic_and_alpha_are_validated();
            rejects_remote_external_and_malformed_sources();
            decodes_embedded_triangle();
        }
    }
}
