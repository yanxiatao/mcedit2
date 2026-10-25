//! Guarded static glTF 2.0 / GLB decoder; only embedded buffers. No fetch,
//! extensions, executable scripts, animations, skinned meshes, or URI traversal.
use crate::special_obj::Triangle;
use serde_json::Value;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_TRIANGLES: usize = 8192;
fn number(v: &Value) -> Result<usize, String> {
    let n = v.as_u64().ok_or("glTF integer required")?;
    usize::try_from(n).map_err(|_| "glTF integer overflow".into())
}
fn field<'a>(v: &'a Value, name: &str) -> Result<&'a Value, String> {
    v.get(name).ok_or_else(|| format!("glTF missing {name}"))
}
fn item<'a>(v: &'a Value, at: usize) -> Result<&'a Value, String> {
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
        if out.len() > MAX_BYTES {
            return Err("glTF buffer exceeds size limit".into());
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
fn accessor<'a>(doc: &Value, bytes: &'a [u8], at: usize) -> Result<Accessor<'a>, String> {
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
    if number(field(view, "buffer")?)? != 0 {
        return Err("Only one embedded glTF buffer supported".into());
    }
    let start = number(view.get("byteOffset").unwrap_or(&Value::Null))
        .unwrap_or(0)
        .checked_add(number(acc.get("byteOffset").unwrap_or(&Value::Null)).unwrap_or(0))
        .ok_or("glTF buffer offset overflow")?;
    let stride = view
        .get("byteStride")
        .map(number)
        .transpose()?
        .unwrap_or(length);
    if stride < length || stride > 256 || count > 65536 {
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
    let view_start = view.get("byteOffset").map(number).transpose()?.unwrap_or(0);
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
                if n.is_finite() && n.abs() <= 256. {
                    Ok(n)
                } else {
                    Err("glTF coordinate not finite/bounded".into())
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
    if !n.is_finite() || n.abs() > 256. {
        return Err("Invalid glTF transform".into());
    }
    Ok(n as f32)
}
fn transform(node: &Value, point: [f32; 3]) -> Result<[f32; 3], String> {
    if let Some(matrix) = node.get("matrix") {
        let m = matrix
            .as_array()
            .filter(|m| m.len() == 16)
            .ok_or("Invalid glTF matrix")?;
        let mut out = [0.; 3];
        for i in 0..3 {
            out[i] = f64_val(m.get(12 + i), 0.)?;
            for j in 0..3 {
                out[i] += f64_val(m.get(4 * j + i), 0.)? * point[j];
            }
        }
        return Ok(out);
    }
    let scale = node.get("scale").and_then(Value::as_array);
    let mut p = point;
    for (i, x) in p.iter_mut().enumerate() {
        *x *= f64_val(scale.and_then(|v| v.get(i)), 1.)?;
    }
    let q = node.get("rotation").and_then(Value::as_array);
    let q = [
        f64_val(q.and_then(|v| v.first()), 0.)?,
        f64_val(q.and_then(|v| v.get(1)), 0.)?,
        f64_val(q.and_then(|v| v.get(2)), 0.)?,
        f64_val(q.and_then(|v| v.get(3)), 1.)?,
    ];
    let t = [
        2. * (q[1] * p[2] - q[2] * p[1]),
        2. * (q[2] * p[0] - q[0] * p[2]),
        2. * (q[0] * p[1] - q[1] * p[0]),
    ];
    for i in 0..3 {
        p[i] += q[3] * t[i] + (q[(i + 1) % 3] * t[(i + 2) % 3] - q[(i + 2) % 3] * t[(i + 1) % 3]);
    }
    let tr = node.get("translation").and_then(Value::as_array);
    for (i, x) in p.iter_mut().enumerate() {
        *x += f64_val(tr.and_then(|v| v.get(i)), 0.)?;
    }
    Ok(p)
}
fn parse_document(doc: Value, data: &[u8]) -> Result<Vec<Triangle>, String> {
    if field(&doc, "asset")?.get("version").and_then(Value::as_str) != Some("2.0") {
        return Err("glTF 2.0 required".into());
    }
    let meshes = field(&doc, "meshes")?
        .as_array()
        .ok_or("Missing glTF meshes")?;
    let nodes = doc.get("nodes").and_then(Value::as_array);
    let mut out = Vec::new();
    // Only explicit mesh nodes are rendered if the scene supplies nodes;
    // without nodes, retain plain mesh asset fallback.
    let instances: Vec<(usize, Option<&Value>)> = if let Some(nodes) = nodes {
        nodes
            .iter()
            .filter_map(|n| {
                n.get("mesh")
                    .and_then(Value::as_u64)
                    .map(|v| (v as usize, Some(n)))
            })
            .collect()
    } else {
        (0..meshes.len()).map(|i| (i, None)).collect()
    };
    if instances.len() > 512 {
        return Err("Too many glTF mesh instances".into());
    }
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
            let positions = accessor(&doc, data, number(field(attrs, "POSITION")?)?)?;
            let uvs = accessor(&doc, data, number(field(attrs, "TEXCOORD_0")?)?)?;
            if uvs.count != positions.count {
                return Err("glTF position/UV count mismatch".into());
            }
            let indices = prim
                .get("indices")
                .map(|v| accessor(&doc, data, number(v)?))
                .transpose()?;
            let count = indices.as_ref().map_or(positions.count, |i| i.count);
            if count % 3 != 0 || out.len() + count / 3 > MAX_TRIANGLES {
                return Err("glTF triangle limit or incomplete primitive".into());
            }
            let material_id = prim.get("material").map(number).transpose()?;
            let material = material_id
                .and_then(|i| doc.get("materials")?.as_array()?.get(i))
                .and_then(|v| v.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("particle");
            if material.len() > 160
                || !material
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            {
                return Err("Invalid glTF material name".into());
            }
            for f in (0..count).step_by(3) {
                let mut xyz = [[0.; 3]; 3];
                let mut uv = [[0.; 2]; 3];
                for k in 0..3 {
                    let i = indices.as_ref().map_or(Ok(f + k), |a| a.index(f + k))?;
                    let position = positions.point(i, 3)?;
                    let point = [position[0], position[1], position[2]];
                    let transformed = if let Some(n) = node {
                        transform(n, point)?
                    } else {
                        point
                    };
                    for c in 0..3 {
                        if !(-1.0..=2.0).contains(&transformed[c]) {
                            return Err("glTF mesh exceeds block bounds".into());
                        }
                        xyz[k][c] = transformed[c] - 0.5;
                    }
                    let coords = uvs.point(i, 2)?;
                    uv[k] = [coords[0], coords[1]];
                }
                out.push(Triangle {
                    xyz,
                    uv,
                    material: material.to_string(),
                });
            }
        }
    }
    if out.is_empty() {
        return Err("glTF has no triangles".into());
    }
    Ok(out)
}
/// Read either GLB 2.0 (embedded BIN chunk) or self-contained glTF JSON with
/// one base64 data URI buffer. External files/network are refused.
pub(crate) fn parse(bytes: &[u8]) -> Result<Vec<Triangle>, String> {
    if bytes.len() > MAX_BYTES {
        return Err("glTF source exceeds 8 MiB".into());
    }
    let (doc, embedded) = if bytes.starts_with(b"glTF") {
        if bytes.len() < 20
            || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) != 2
            || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize != bytes.len()
        {
            return Err("Invalid GLB 2.0 header".into());
        }
        let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        if len > bytes.len() - 20 || &bytes[16..20] != b"JSON" {
            return Err("Missing GLB JSON chunk".into());
        }
        let json: Value =
            serde_json::from_slice(&bytes[20..20 + len]).map_err(|e| e.to_string())?;
        let offset = 20 + len;
        if bytes.len() < offset + 8 || &bytes[offset + 4..offset + 8] != b"BIN\0" {
            return Err("Missing GLB BIN chunk".into());
        }
        let size = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        if size > bytes.len() - offset - 8 {
            return Err("GLB BIN exceeds file".into());
        }
        (json, bytes[offset + 8..offset + 8 + size].to_vec())
    } else {
        let json: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let buffer = item(field(&json, "buffers")?, 0)?;
        let uri = field(buffer, "uri")?
            .as_str()
            .ok_or("glTF buffer URI not a string")?;
        let encoded = uri
            .strip_prefix("data:application/octet-stream;base64,")
            .or_else(|| uri.strip_prefix("data:application/gltf-buffer;base64,"))
            .ok_or("External glTF buffer URI not supported")?;
        let blob = decode_base64(encoded)?;
        (json, blob)
    };
    let declared = number(field(item(field(&doc, "buffers")?, 0)?, "byteLength")?)?;
    if declared > embedded.len() {
        return Err("glTF declared buffer exceeds source bytes".into());
    }
    parse_document(doc, &embedded)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_remote_external_and_malformed_sources() {
        assert!(parse(
            br#"{"asset":{"version":"2.0"},"buffers":[{"uri":"https://remote","byteLength":4}]}"#
        )
        .is_err());
        assert!(parse(b"glTFbad").is_err());
        assert!(parse(br#"{"asset":{"version":"1.0"}}"#).is_err());
    }
    #[test]
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
        while json.len() % 4 != 0 {
            json.push(b' ')
        }
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(12 + 8 + json.len() + 8 + bin.len() as usize).to_le_bytes()[..4]);
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
    }
}
