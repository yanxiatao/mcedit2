//! Bounded lookup of resource-pack blockstate variants and model texture maps.
//! A small conservative subset of Java JSON model inheritance is supported;
//! unsupported multipart, rotations, random weighted variants and custom
//! geometry use the ordinary named-texture fallback rather than guessing.
use crate::{isometric::Face, resource_ids};
use crate::models::{Box3, ElementRotation, Part};
use crate::{special_obj, special_gltf, special_mtl};
use mcedit2_world::sections::State;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FaceSpec {
    pub texture: String,
    /// Minecraft model-element UV rectangle, in 0..16 texture units.
    pub uv: [f32; 4],
    pub rotation: u16,
    pub tint_index: Option<u32>,
    /// Source face vertex i samples base texture corner uvlock[i].
    /// Handles quarter-turns and mirrors without corrupting geometry.
    pub uvlock: [u8; 4],
}
#[derive(Clone, Debug)]
pub(crate) struct ObjSelection {
    pub xyz: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub texture: String,
    pub tint: [f32; 4],
    pub alpha_mode: Option<String>,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct ModelBank {
    files: BTreeMap<String, Value>,
    obj_meshes: BTreeMap<String, Vec<special_obj::Triangle>>,
    gltf_meshes: BTreeMap<String, Vec<special_obj::Triangle>>,
    gltf_materials: BTreeMap<String, BTreeMap<String, special_gltf::StaticPbrMaterial>>,
    gltf_sources: BTreeMap<String, (String, Vec<u8>)>,
    gltf_buffers: BTreeMap<String, Vec<u8>>,
    gltf_buffer_bytes: usize,
    obj_materials: BTreeMap<String, BTreeMap<String, special_mtl::Material>>,
    obj_mask_overlays: BTreeMap<(String, String), String>,
    json_sizes: BTreeMap<String, usize>,
    total_json_bytes: usize,
}
fn asset_id(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 4 || parts[0] != "assets" {
        return None;
    }
    let namespace = parts[1];
    if !resource_ids::namespace(namespace) {
        return None;
    }
    let (kind, start) = match parts.get(2).copied() {
        Some("blockstates") => ("state", 3),
        Some("models") if parts.len() >= 5 => ("model", 3),
        _ => return None,
    };
    let relative = parts[start..].join("/");
    let id = relative.strip_suffix(".json")?;
    // block/ IDs historically omit their parent folder. Preserve every
    // other model family to avoid item/entity/custom name collisions.
    let id = if kind == "model" {
        id.strip_prefix("block/").unwrap_or(id)
    } else {
        id
    };
    resource_ids::path(id, 160).then(|| format!("{kind}:{namespace}:{id}"))
}
fn normalize_model(value: &str, default_ns: &str) -> Option<String> {
    let id = resource_ids::qualified(value, default_ns)?;
    let (namespace, path) = id.split_once(':')?;
    let path = path.strip_prefix("block/").unwrap_or(path);
    resource_ids::path(path, 160).then(|| format!("model:{namespace}:{path}"))
}
/// Vanilla's common block parent templates are not bundled in most
/// resource packs. Resolve only known minecraft: parent IDs to trusted,
/// bounded model elements, preserving all child texture aliases. Never
/// synthesize geometry for an absent third-party namespace/parent.
fn vanilla_parent_elements(id: &str) -> Option<&'static Vec<Value>> {
    static TEMPLATES: OnceLock<BTreeMap<&'static str, Vec<Value>>> = OnceLock::new();
    let templates = TEMPLATES.get_or_init(|| {
        const DIRS: [&str; 6] = ["down", "up", "north", "south", "west", "east"];
        let mut out = BTreeMap::new();
        for (name, aliases) in [
            ("cube_all", ["all", "all", "all", "all", "all", "all"]),
            ("cube_top", ["side", "top", "side", "side", "side", "side"]),
            ("cube", ["down", "up", "north", "south", "west", "east"]),
            (
                "cube_column",
                ["end", "end", "side", "side", "side", "side"],
            ),
            (
                "cube_column_horizontal",
                ["side", "side", "end", "end", "side", "side"],
            ),
            (
                "cube_bottom_top",
                ["bottom", "top", "side", "side", "side", "side"],
            ),
            (
                "orientable",
                ["bottom", "top", "front", "side", "side", "side"],
            ),
            (
                "orientable_with_bottom",
                ["bottom", "top", "front", "side", "side", "side"],
            ),
        ] {
            let mut faces = serde_json::Map::new();
            for (dir, alias) in DIRS.into_iter().zip(aliases) {
                faces.insert(
                    dir.to_string(),
                    serde_json::json!({
                        "texture":format!("#{alias}"),"cullface":dir
                    }),
                );
            }
            out.insert(
                name,
                vec![serde_json::json!({
                    "from":[0,0,0],"to":[16,16,16],"faces":faces
                })],
            );
        }
        // A number of vanilla leaf variants inherit a tinted full cube.
        // Only these explicit trusted vanilla parents receive tintindex;
        // modded cubes cannot be assumed biome-tinted.
        let mut leaves_faces = serde_json::Map::new();
        for direction in DIRS {
            leaves_faces.insert(
                direction.into(),
                serde_json::json!({
                    "texture":"#all", "cullface":direction, "tintindex":0
                }),
            );
        }
        out.insert(
            "leaves",
            vec![serde_json::json!({
                "from":[0,0,0],"to":[16,16,16],"faces":leaves_faces
            })],
        );
        // Vanilla fence sub-models are frequently absent from small override
        // packs. Recover their documented 4x4 post and two slender 2x3 rails
        // (rather than showing a full missing-texture cube).
        let fence_box = |from: [u8; 3], to: [u8; 3]| {
            let mut faces = serde_json::Map::new();
            for direction in DIRS {
                faces.insert(
                    direction.into(),
                    serde_json::json!({
                        "texture":"#texture"
                    }),
                );
            }
            serde_json::json!({"from":from,"to":to,"faces":faces})
        };
        out.insert(
            "template_fence_post",
            vec![fence_box([6, 0, 6], [10, 16, 10])],
        );
        out.insert(
            "template_fence_side",
            vec![
                fence_box([7, 6, 0], [9, 9, 9]),
                fence_box([7, 12, 0], [9, 15, 9]),
            ],
        );
        // Crossed plants are two physically flat, double-sided planes.
        // Unlike a fallback cube they admit transparent holes and accurate
        // silhouette/picking under a JSON model with no copied parent.
        let cross = |tinted: bool| {
            [45, -45]
                .into_iter()
                .map(|angle| {
                    serde_json::json!({
                        "from":[0,0,8],"to":[16,16,8],
                        "rotation":{"origin":[8,8,8],"axis":"y","angle":angle},
                        "shade":false,
                        "faces":{"north":{"texture":"#cross"},
                                 "south":{"texture":"#cross"}}
                    })
                })
                .map(|mut element| {
                    if tinted {
                        if let Some(faces) = element.get_mut("faces").and_then(Value::as_object_mut)
                        {
                            for face in faces.values_mut() {
                                face["tintindex"] = serde_json::json!(0);
                            }
                        }
                    }
                    element
                })
                .collect::<Vec<_>>()
        };
        out.insert("cross", cross(false));
        out.insert("tinted_cross", cross(true));
        out
    });
    let id = id.strip_prefix("model:minecraft:")?;
    templates.get(id)
}
fn normalize_texture(value: &str, default_ns: &str) -> Option<String> {
    let id = resource_ids::qualified(value, default_ns)?;
    let (namespace, path) = id.split_once(':')?;
    let path = path.strip_prefix("block/").unwrap_or(path);
    resource_ids::path(path, 160).then(|| format!("{namespace}:{path}"))
}
fn variant_matches(name: &str, state: &State) -> bool {
    name.split(',').all(|entry| {
        if entry.is_empty() {
            return true;
        }
        let Some((key, value)) = entry.split_once('=') else {
            return false;
        };
        state.properties.iter().any(|(k, v)| k == key && v == value)
    })
}
fn best_variant<'a>(obj: &'a serde_json::Map<String, Value>, state: &State) -> Option<&'a Value> {
    obj.iter()
        .filter(|(k, _)| variant_matches(k, state))
        .max_by_key(|(k, _)| k.split(',').filter(|s| !s.is_empty()).count())
        .map(|(_, v)| v)
}
fn variant_angles(value: &Value) -> Option<(i32, i32)> {
    if value.is_array() || value.get("uvlock").is_some_and(|v| v.as_bool().is_none()) {
        return None;
    }
    let angle = |key: &str| -> Option<i32> {
        let value = value.get(key);
        let n = if let Some(v) = value { v.as_i64()? } else { 0 };
        if n.abs() > 360 || n % 90 != 0 {
            return None;
        }
        i32::try_from(n).ok()
    };
    Some((angle("x")?, angle("y")?))
}
#[derive(Debug, Clone, Hash)]
struct ChosenModel {
    key: String,
    x: i32,
    y: i32,
    uvlock: bool,
}
fn deterministic_model_seed(state: &State, pos: (i32, i32, i32), salt: u64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state.name.hash(&mut hasher);
    state.properties.hash(&mut hasher);
    pos.hash(&mut hasher);
    salt.hash(&mut hasher);
    hasher.finish()
}
/// Choose a weighted Java model entry without a global RNG: reloading a
/// chunk/camera yields the same source XYZ appearance and matched UVs.
fn choose_weighted<'a>(value: &'a Value, seed: u64) -> Option<&'a Value> {
    let Some(array) = value.as_array() else {
        return Some(value);
    };
    if array.is_empty() || array.len() > 64 {
        return None;
    }
    let mut weights = Vec::with_capacity(array.len());
    let mut total = 0u64;
    for item in array {
        let weight = item.get("weight").map(Value::as_u64).unwrap_or(Some(1))?;
        if weight == 0 || weight > 10_000 {
            return None;
        }
        total = total.checked_add(weight)?;
        if total > 640_000 {
            return None;
        }
        weights.push(weight);
    }
    let mut target = seed % total;
    for (item, weight) in array.iter().zip(weights) {
        if target < weight {
            return Some(item);
        }
        target -= weight;
    }
    None
}
fn uvlock_permutation(local: Face, x: i32, y: i32) -> Option<[u8; 4]> {
    const FACES: [Face; 6] = [
        Face::Top,
        Face::Bottom,
        Face::PosX,
        Face::NegX,
        Face::PosZ,
        Face::NegZ,
    ];
    let part = Part {
        bounds: Box3::FULL,
        rotation: None,
        variant_x: x,
        variant_y: y,
    };
    let normal = part.normal(local.normal());
    let target = FACES.iter().copied().find(|face| {
        let n = face.normal();
        let expected = [n.0 as f32, n.1 as f32, n.2 as f32];
        normal
            .iter()
            .zip(expected)
            .all(|(a, b)| (*a - b).abs() < 1e-5)
    })?;
    let rotated = part.face_vertices(local.corners());
    let canonical = target.corners();
    let mut permutation = [0u8; 4];
    for (i, point) in rotated.iter().enumerate() {
        let index = canonical
            .iter()
            .position(|target| point.iter().zip(target).all(|(a, b)| (*a - b).abs() < 1e-5))?;
        permutation[i] = index as u8;
    }
    let mut sorted = permutation;
    sorted.sort_unstable();
    (sorted == [0, 1, 2, 3]).then_some(permutation)
}

/// Minecraft multipart: property AND, pipe-separated alternatives,
/// nested OR/AND, and omitted when (always). Reject unknown syntax.
fn condition_matches(condition: &Value, state: &State, depth: usize) -> Option<bool> {
    if depth > 8 {
        return None;
    }
    let terms = condition.as_object()?;
    if terms.len() > 64 {
        return None;
    }
    let mut all = true;
    for (key, value) in terms {
        let matches = match key.as_str() {
            "OR" | "AND" => {
                let expressions = value.as_array()?;
                if expressions.is_empty() || expressions.len() > 64 {
                    return None;
                }
                let mut any = false;
                let mut every = true;
                for expr in expressions {
                    let yes = condition_matches(expr, state, depth + 1)?;
                    any |= yes;
                    every &= yes;
                }
                if key == "OR" {
                    any
                } else {
                    every
                }
            }
            _ => {
                if key.is_empty()
                    || key.len() > 80
                    || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                {
                    return None;
                }
                let allowed = value.as_str()?;
                let actual = state
                    .properties
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.as_str());
                let mut any = false;
                for alt in allowed.split('|') {
                    if alt.is_empty() {
                        return None;
                    }
                    any |= actual == Some(alt);
                }
                any
            }
        };
        all &= matches;
    }
    Some(all)
}
fn read_rotation_origin(doc: &Value) -> Option<[f32; 3]> {
    let v = doc.get("origin")?.as_array()?;
    if v.len() != 3 {
        return None;
    }
    let mut o = [0.0; 3];
    for (i, x) in v.iter().enumerate() {
        let value = x.as_f64()? as f32;
        if !value.is_finite() || !(-16.0..=32.0).contains(&value) {
            return None;
        }
        o[i] = value / 16.0 - 0.5;
    }
    Some(o)
}
/// Implicit Java model element UVs depend on which face of a partial
/// cuboid is rendered. A full [0,0,16,16] texture stretches partial models.
fn implicit_face_uv(element: &Value, face: Face) -> Option<[f32; 4]> {
    let parse = |name: &str| -> Option<[f32; 3]> {
        let coords = element.get(name)?.as_array()?;
        if coords.len() != 3 {
            return None;
        }
        let mut out = [0f32; 3];
        for (i, v) in coords.iter().enumerate() {
            out[i] = v.as_f64()? as f32;
            if !out[i].is_finite() || !(-16.0..=32.0).contains(&out[i]) {
                return None;
            }
        }
        Some(out)
    };
    let [x0, y0, z0] = parse("from")?;
    let [x1, y1, z1] = parse("to")?;
    if x0 > x1 || y0 > y1 || z0 > z1 {
        return None;
    }
    let flat = [x0 == x1, y0 == y1, z0 == z1];
    if flat.iter().filter(|&&v| v).count() > 1 {
        return None;
    }
    if flat[0] && !matches!(face, Face::NegX | Face::PosX)
        || flat[1] && !matches!(face, Face::Bottom | Face::Top)
        || flat[2] && !matches!(face, Face::NegZ | Face::PosZ)
    {
        return None;
    }
    Some(match face {
        Face::Bottom => [x0, 16.0 - z1, x1, 16.0 - z0],
        Face::Top => [x0, z0, x1, z1],
        Face::NegZ => [16.0 - x1, 16.0 - y1, 16.0 - x0, 16.0 - y0],
        Face::PosZ => [x0, 16.0 - y1, x1, 16.0 - y0],
        Face::NegX => [z0, 16.0 - y1, z1, 16.0 - y0],
        Face::PosX => [16.0 - z1, 16.0 - y1, 16.0 - z0, 16.0 - y0],
    })
}
fn variant_model(value: &Value) -> Option<&str> {
    // Deterministic first entry for weighted variants; no RNG or invented geometry.
    let value = if let Some(list) = value.as_array() {
        list.first()?
    } else {
        value
    };
    value.get("model")?.as_str()
}
fn face_keys(face: Face, parent: &str) -> &'static [&'static str] {
    const TOP: &[&str] = &["up", "top", "end", "all"];
    const BOTTOM: &[&str] = &["down", "bottom", "end", "all"];
    const NORTH: &[&str] = &["north", "side", "all"];
    const SOUTH: &[&str] = &["south", "side", "all"];
    const EAST: &[&str] = &["east", "side", "all"];
    const WEST: &[&str] = &["west", "side", "all"];
    // "parent" retained for future precise parent geometry expansion.
    let _ = parent;
    match face {
        Face::Top => TOP,
        Face::Bottom => BOTTOM,
        Face::PosX => EAST,
        Face::NegX => WEST,
        Face::PosZ => SOUTH,
        Face::NegZ => NORTH,
    }
}
impl ModelBank {
    fn selections(&self, state: &State) -> Option<Vec<ChosenModel>> {
        self.selections_at(state, (0, 0, 0))
    }
    fn selections_at(&self, state: &State, pos: (i32, i32, i32)) -> Option<Vec<ChosenModel>> {
        let (ns, name) = state.name.split_once(':')?;
        let doc = self.files.get(&format!("state:{ns}:{name}"))?;
        if let Some(parts) = doc.get("multipart") {
            let parts = parts.as_array()?;
            if parts.is_empty() || parts.len() > 64 {
                return None;
            }
            let mut chosen = Vec::new();
            for (index, part) in parts.iter().enumerate() {
                let active = part
                    .get("when")
                    .map(|v| condition_matches(v, state, 0))
                    .unwrap_or(Some(true))?;
                if !active {
                    continue;
                }
                let apply = choose_weighted(
                    part.get("apply")?,
                    deterministic_model_seed(state, pos, index as u64 + 1),
                )?;
                let (x, y) = variant_angles(apply)?;
                chosen.push(ChosenModel {
                    key: normalize_model(apply.get("model")?.as_str()?, ns)?,
                    x,
                    y,
                    uvlock: apply
                        .get("uvlock")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
            if chosen.is_empty() {
                return None;
            }
            return Some(chosen);
        }
        let value = choose_weighted(
            best_variant(doc.get("variants")?.as_object()?, state)?,
            deterministic_model_seed(state, pos, 0),
        )?;
        let (x, y) = variant_angles(value)?;
        Some(vec![ChosenModel {
            key: normalize_model(value.get("model")?.as_str()?, ns)?,
            x,
            y,
            uvlock: value
                .get("uvlock")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }])
    }
    fn chosen_element(&self, state: &State, index: usize) -> Option<(ChosenModel, &Value)> {
        self.chosen_element_at(state, index, (0, 0, 0))
    }
    fn chosen_element_at(
        &self,
        state: &State,
        index: usize,
        pos: (i32, i32, i32),
    ) -> Option<(ChosenModel, &Value)> {
        if index >= 512 {
            return None;
        }
        let mut remaining = index;
        for model in self.selections_at(state, pos)? {
            let elements = self.model_elements(&model.key)?;
            if remaining < elements.len() {
                return Some((model, elements.get(remaining)?));
            }
            remaining = remaining.checked_sub(elements.len())?;
        }
        None
    }
    /// Signed block position changes weighted variants; the cache key
    /// hashes only the chosen model set, not each individual coordinate.
    pub fn weighted_selection_key(&self, state: &State, pos: (i32, i32, i32)) -> u64 {
        let Some(chosen) = self.selections_at(state, pos) else {
            return 0;
        };
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        chosen.hash(&mut hash);
        hash.finish()
    }
    /// Elements may deliberately disable directional shading, notably the
    /// zero-thickness crossed planes used by flowers and saplings.
    pub fn element_unshaded(&self, state: &State, index: usize) -> Option<bool> {
        self.element_unshaded_at(state, index, (0, 0, 0))
    }
    pub fn element_unshaded_at(
        &self,
        state: &State,
        index: usize,
        pos: (i32, i32, i32),
    ) -> Option<bool> {
        let (_, element) = self.chosen_element_at(state, index, pos)?;
        let shade = element
            .get("shade")
            .map(Value::as_bool)
            .unwrap_or(Some(true))?;
        Some(!shade)
    }
    /// Some(false) means an explicit Java model face is absent and must NOT
    /// be rendered as a synthetic palette-textured side. None means that
    /// the model/element is not resolved and the fallback remains active.
    pub fn face_defined(&self, state: &State, face: Face, index: usize) -> Option<bool> {
        self.face_defined_at(state, face, index, (0, 0, 0))
    }
    pub fn face_defined_at(
        &self,
        state: &State,
        face: Face,
        index: usize,
        pos: (i32, i32, i32),
    ) -> Option<bool> {
        let (_model, element) = self.chosen_element_at(state, index, pos)?;
        let faces = element.get("faces")?.as_object()?;
        let name = match face {
            Face::Top => "up",
            Face::Bottom => "down",
            Face::PosX => "east",
            Face::NegX => "west",
            Face::PosZ => "south",
            Face::NegZ => "north",
        };
        Some(faces.contains_key(name))
    }
    /// Return the rotated, *world-facing* vanilla cullface declaration.
    /// Some(None) is an explicitly rendered face without cullface: it must
    /// not be removed just because an adjacent block fills the cell.
    /// None means no reliable JSON face metadata was found.
    pub fn face_cull_direction(
        &self,
        state: &State,
        face: Face,
        index: usize,
    ) -> Option<Option<Face>> {
        self.face_cull_direction_at(state, face, index, (0, 0, 0))
    }
    pub fn face_cull_direction_at(
        &self,
        state: &State,
        face: Face,
        index: usize,
        pos: (i32, i32, i32),
    ) -> Option<Option<Face>> {
        let (chosen, element) = self.chosen_element_at(state, index, pos)?;
        let name = match face {
            Face::Top => "up",
            Face::Bottom => "down",
            Face::PosX => "east",
            Face::NegX => "west",
            Face::PosZ => "south",
            Face::NegZ => "north",
        };
        let face_doc = element.get("faces")?.get(name)?;
        let Some(raw) = face_doc.get("cullface") else {
            return Some(None);
        };
        let local = match raw.as_str()? {
            "up" => Face::Top,
            "down" => Face::Bottom,
            "east" => Face::PosX,
            "west" => Face::NegX,
            "south" => Face::PosZ,
            "north" => Face::NegZ,
            _ => return None,
        };
        let mut part = Part::cube(Box3::FULL);
        part.variant_x = chosen.x;
        part.variant_y = chosen.y;
        let n = part.normal(local.normal());
        let rotated = match n {
            [1.0, 0.0, 0.0] => Face::PosX,
            [-1.0, 0.0, 0.0] => Face::NegX,
            [0.0, 1.0, 0.0] => Face::Top,
            [0.0, -1.0, 0.0] => Face::Bottom,
            [0.0, 0.0, 1.0] => Face::PosZ,
            [0.0, 0.0, -1.0] => Face::NegZ,
            _ => return None,
        };
        Some(Some(rotated))
    }
    fn model_elements(&self, key: &str) -> Option<&Vec<Value>> {
        let mut current = Some(key.to_string());
        let mut seen = BTreeSet::new();
        while let Some(id) = current {
            if seen.len() >= 16 || !seen.insert(id.clone()) {
                return None;
            }
            let Some(model) = self.files.get(&id) else {
                return vanilla_parent_elements(&id);
            };
            if let Some(elements) = model.get("elements") {
                return elements.as_array();
            }
            let source_ns = id.strip_prefix("model:")?.split_once(':')?.0;
            current = model
                .get("parent")
                .and_then(Value::as_str)
                .and_then(|v| normalize_model(v, source_ns));
        }
        None
    }
    pub fn insert(&mut self, path: &str, bytes: &[u8]) -> Result<bool, String> {
        let Some(key) = asset_id(path) else {
            return Ok(false);
        };
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("Individual model JSON exceeds 2MiB".into());
        }
        // No fixed model *count*: enormous modpacks are valid. A single
        // source JSON stays <=256KiB, and aggregate parsed source is
        // guarded against decompression/memory denial of service.
        const JSON_BUDGET: usize = 256 * 1024 * 1024;
        let old = self.json_sizes.get(&key).copied().unwrap_or(0);
        let new_total = self
            .total_json_bytes
            .saturating_sub(old)
            .checked_add(bytes.len())
            .ok_or("Model JSON aggregate size overflow")?;
        if new_total > JSON_BUDGET {
            return Err("Model JSON source exceeds 256MiB safety budget".into());
        }
        let json: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !json.is_object() {
            return Err("Model JSON root must be an object".into());
        }
        self.json_sizes.insert(key.clone(), bytes.len());
        self.total_json_bytes = new_total;
        self.files.insert(key, json);
        Ok(true)
    }
    /// Guarded static Wavefront OBJ; model key uses original Java resource ID.
    pub fn insert_obj(&mut self, path: &str, bytes: &[u8]) -> Result<bool, String> {
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() < 5
            || parts[0] != "assets"
            || parts[2] != "models"
            || !resource_ids::namespace(parts[1])
        {
            return Ok(false);
        }
        let name = parts[3..].join("/");
        let Some(name) = name.strip_suffix(".obj") else {
            return Ok(false);
        };
        if !resource_ids::path(name, 160) {
            return Ok(false);
        }
        let key = format!(
            "model:{}:{}",
            parts[1],
            name.strip_prefix("block/").unwrap_or(name)
        );
        let mesh = special_obj::parse(bytes)?;
        self.obj_meshes.insert(key, mesh);
        Ok(true)
    }
    /// Each OBJ may use an optional same-stem MTL from the same pack.
    /// No external mtllib reference is followed.
    pub fn insert_mtl(&mut self, path: &str, bytes: &[u8]) -> Result<bool, String> {
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() < 5
            || parts[0] != "assets"
            || parts[2] != "models"
            || !resource_ids::namespace(parts[1])
        {
            return Ok(false);
        }
        let name = parts[3..].join("/");
        let Some(name) = name.strip_suffix(".mtl") else {
            return Ok(false);
        };
        if !resource_ids::path(name, 160) {
            return Ok(false);
        }
        let key = format!(
            "model:{}:{}",
            parts[1],
            name.strip_prefix("block/").unwrap_or(name)
        );
        self.obj_materials.insert(key, special_mtl::parse(bytes)?);
        Ok(true)
    }
    /// Static glTF / GLB source meshes share the same vertex material pipeline
    /// as OBJ. All buffers must be embedded; there is no network/file resolver.
    pub fn insert_gltf(&mut self, path: &str, bytes: &[u8]) -> Result<bool, String> {
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() < 5
            || parts[0] != "assets"
            || parts[2] != "models"
            || !resource_ids::namespace(parts[1])
        {
            return Ok(false);
        }
        let name = parts[3..].join("/");
        let name = name
            .strip_suffix(".gltf")
            .or_else(|| name.strip_suffix(".glb"));
        let Some(name) = name else {
            return Ok(false);
        };
        if !resource_ids::path(name, 160) {
            return Ok(false);
        }
        let key = format!(
            "model:{}:{}",
            parts[1],
            name.strip_prefix("block/").unwrap_or(name)
        );
        // Defer glTF decoding until the complete pack stack is ingested.
        // A .bin member may precede or follow the referring model in ZIPs.
        self.gltf_sources.insert(key, (path.to_owned(), bytes.to_vec()));
        Ok(true)
    }
    /// External .bin blobs are only resolved as exact, same-pack resource
    /// members after loading all layers. They are never opened by URI.
    pub fn insert_gltf_buffer(&mut self, path:&str, bytes:&[u8])->Result<bool,String>{
        let parts:Vec<_>=path.split('/').collect();
        if parts.len()<5 || parts[0]!="assets" || parts[2]!="models"
            || !resource_ids::namespace(parts[1]) {return Ok(false);}
        let relative=parts[3..].join("/");
        if !relative.ends_with(".bin") || !resource_ids::path(&relative,160){
            return Ok(false);
        }
        if bytes.len()>8*1024*1024{return Err("glTF buffer exceeds 8 MiB".into());}
        let old=self.gltf_buffers.get(path).map_or(0,Vec::len);
        let total=self.gltf_buffer_bytes.saturating_sub(old)
            .checked_add(bytes.len()).ok_or("glTF buffer total overflow")?;
        if total>128*1024*1024{return Err("glTF buffer total exceeds 128 MiB".into());}
        self.gltf_buffer_bytes=total;
        self.gltf_buffers.insert(path.to_owned(),bytes.to_vec());
        Ok(true)
    }
    /// Compile loaded models only when all resource pack layers are present.
    /// Missing/unsupported models are skipped with an explicit diagnostic.
    pub fn finish_gltf(&mut self)->Vec<String>{
        let mut errors=Vec::new();
        self.gltf_meshes.clear();
        self.gltf_materials.clear();
        for (id,(source,bytes)) in &self.gltf_sources{
            let folder=source.rsplit_once('/').map(|(dir,_)|dir).unwrap_or("");
            let resolved=special_gltf::parse_with_resolver(bytes,&|uri|{
                // The decoder has already validated relative URI components.
                self.gltf_buffers.get(&format!("{folder}/{uri}")).cloned()
            });
            match resolved{
                Ok(mesh)=>{
                    match special_gltf::material_factors(bytes){
                        Ok(mat)=>{self.gltf_materials.insert(id.clone(),mat);}
                        Err(e)=>{errors.push(format!("{source}: {e}"));continue;}
                    }
                    self.gltf_meshes.insert(id.clone(),mesh);
                }
                Err(e)=>errors.push(format!("{source}: {e}")),
            }
        }
        errors
    }
    /// List exact original MTL opacity-map pairs. Resource decoding and
    /// overlay allocation belong to the atlas, never to model geometry.
    pub fn opacity_maps(&self)->Vec<(String,String,String,String)>{
        let mut entries=Vec::new();
        for (model,materials) in &self.obj_materials{
            for (name,mat) in materials{
                if let (Some(diffuse),Some(mask))=(&mat.map_kd,&mat.map_d){
                    entries.push((model.clone(),name.clone(),diffuse.clone(),mask.clone()));
                }
            }
        }
        entries
    }
    pub fn set_opacity_overlay(&mut self,model:String,name:String,sprite:String){
        self.obj_mask_overlays.insert((model,name),sprite);
    }
    /// Select special models through the original blockstate JSON. Loader
    /// declarations are a data contract, not an arbitrary executable hook.
    pub fn obj_at(&self, state: &State, pos: (i32, i32, i32)) -> Option<Vec<ObjSelection>> {
        let mut out = Vec::new();
        let selected = self.selections_at(state, pos)?;
        for chosen in selected {
            let model = self.files.get(&chosen.key)?;
            // Any Mod-defined client-only loader can opt into a static,
            // declarative fallback without running its executable renderer.
            // Unknown loaders without an explicit adapter remain unsupported.
            let adapter = model.get("mcedit2:static_renderer");
            let suffix = if let Some(adapter) = adapter {
                match adapter.get("format")?.as_str()? {
                    "obj" => ".obj",
                    "gltf" => ".gltf",
                    "glb" => ".glb",
                    _ => return None,
                }
            } else {
                match model.get("loader")?.as_str()? {
                    "forge:obj" | "neoforge:obj" => ".obj",
                    "mcedit2:gltf" | "forge:gltf" | "neoforge:gltf" => ".gltf",
                    "mcedit2:glb" | "forge:glb" | "neoforge:glb" => ".glb",
                    _ => return None,
                }
            };
            let source_ns = chosen.key.strip_prefix("model:")?.split_once(':')?.0;
            let reference = resource_ids::qualified(
                adapter
                    .and_then(|a| a.get("model"))
                    .or_else(|| model.get("model"))?
                    .as_str()?,
                source_ns,
            )?;
            let (ns, path) = reference.split_once(':')?;
            let path = path.strip_prefix("models/").unwrap_or(path);
            let path = path.strip_suffix(suffix)?;
            let path = path.strip_prefix("block/").unwrap_or(path);
            let id = format!("model:{ns}:{path}");
            let mesh = if suffix == ".obj" {
                self.obj_meshes.get(&id)?
            } else {
                self.gltf_meshes.get(&id)?
            };
            let rotation = Part {
                bounds: Box3::FULL,
                rotation: None,
                variant_x: chosen.x,
                variant_y: chosen.y,
            };
            let mappings = adapter
                .and_then(|a| a.get("textures"))
                .or_else(|| model.get("textures"))
                .and_then(Value::as_object);
            let materials = if suffix == ".obj" {
                self.obj_materials.get(&id)
            } else {
                None
            };
            let id_key=id.clone();
            let gltf_materials=self.gltf_materials.get(&id);
            for tri in mesh {
                let mtl = materials.and_then(|m| m.get(&tri.material));
                let pbr=gltf_materials.and_then(|m|m.get(&tri.material));
                let raw = mappings
                    .and_then(|m| m.get(&tri.material))
                    .and_then(Value::as_str)
                    .or_else(|| mtl.and_then(|m| m.map_kd.as_deref()))
                    .or_else(|| {
                        mappings
                            .and_then(|m| m.get("particle"))
                            .and_then(Value::as_str)
                    })?;
                let texture = resource_ids::qualified(raw, source_ns)?;
                let (ns, path) = texture.split_once(':')?;
                let id = path
                    .strip_prefix("block/")
                    .or_else(|| path.strip_prefix("blocks/"))
                    .unwrap_or(path);
                out.push(ObjSelection {
                    xyz: tri.xyz.map(|p| rotation.point(p)),
                    uv: tri.uv,
                    texture: self.obj_mask_overlays
                        .get(&(id_key.clone(),tri.material.clone()))
                        .cloned()
                        .unwrap_or_else(||format!("{ns}:{id}")),
                    tint: mtl.map(|m| m.diffuse).or_else(|| pbr.map(|m| m.base_color))
                        .unwrap_or([1.; 4]),
                    alpha_mode: pbr.map(|m| m.alpha_mode.clone()),
                    alpha_cutoff: pbr.map_or(0.5, |m| m.alpha_cutoff),
                    double_sided: pbr.is_none_or(|m| m.double_sided),
                });
                if out.len() > 4096 {
                    return None;
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }
    pub fn count(&self) -> usize {
        self.files.len()
    }
    /// Flatten all matched variant/multipart model elements while retaining
    /// original voxel identity for picking.
    /// Native visual parts support vanilla 22.5/45 degree element axes
    /// without collapsing the source geometry into a larger AABB.
    pub fn parts(&self, state: &State) -> Option<Vec<Part>> {
        self.parts_at(state, (0, 0, 0))
    }
    pub fn parts_at(&self, state: &State, pos: (i32, i32, i32)) -> Option<Vec<Part>> {
        let chosen = self.selections_at(state, pos)?;
        let mut out = Vec::new();
        for model in chosen {
            let elements = self.model_elements(&model.key)?;
            if elements.is_empty() || elements.len() > 256 || out.len() + elements.len() > 512 {
                return None;
            }
            for entry in elements {
                let read = |name: &str| -> Option<[f32; 3]> {
                    let a = entry.get(name)?.as_array()?;
                    if a.len() != 3 {
                        return None;
                    }
                    Some([
                        a[0].as_f64()? as f32,
                        a[1].as_f64()? as f32,
                        a[2].as_f64()? as f32,
                    ])
                };
                let from = read("from")?;
                let to = read("to")?;
                let bounds = Box3::from_model_units(from, to).or_else(|| {
                    let flat = Box3::from_model_units_plane(from, to)?;
                    let axis = (0..3).find(|&i| from[i] == to[i])?;
                    let sides = match axis {
                        0 => ["east", "west"],
                        1 => ["up", "down"],
                        _ => ["north", "south"],
                    };
                    let faces = entry.get("faces")?.as_object()?;
                    // The zero-thickness plane is renderable only when the
                    // JSON explicitly declares a face normal to its axis.
                    sides
                        .iter()
                        .any(|name| faces.contains_key(*name))
                        .then_some(flat)
                })?;
                let rotation = if let Some(doc) = entry.get("rotation") {
                    let axis = match doc.get("axis")?.as_str()? {
                        "x" => 0,
                        "y" => 1,
                        "z" => 2,
                        _ => return None,
                    };
                    let angle = doc.get("angle")?.as_f64()? as f32;
                    if ![-45.0, -22.5, 0.0, 22.5, 45.0].contains(&angle) {
                        return None;
                    }
                    let o = read_rotation_origin(doc)?;
                    Some(ElementRotation {
                        axis,
                        origin: o,
                        angle_degrees: angle,
                        rescale: doc.get("rescale").and_then(Value::as_bool).unwrap_or(false),
                    })
                } else {
                    None
                };
                out.push(Part {
                    bounds,
                    rotation,
                    variant_x: model.x,
                    variant_y: model.y,
                });
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
    pub fn elements(&self, state: &State) -> Option<Vec<Box3>> {
        let chosen = self.selections(state)?;
        let mut out = Vec::new();
        for model in chosen {
            let elements = self.model_elements(&model.key)?;
            if elements.is_empty() || elements.len() > 256 || out.len() + elements.len() > 512 {
                return None;
            }
            for entry in elements {
                if entry.get("rotation").is_some() {
                    return None;
                }
                let read = |name: &str| -> Option<[f32; 3]> {
                    let a = entry.get(name)?.as_array()?;
                    if a.len() != 3 {
                        return None;
                    }
                    Some([
                        a[0].as_f64()? as f32,
                        a[1].as_f64()? as f32,
                        a[2].as_f64()? as f32,
                    ])
                };
                out.push(
                    Box3::from_model_units(read("from")?, read("to")?)?.rotate(model.x, model.y)?,
                );
            }
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
    /// A specific Java model element face's texture and declared UV.
    /// This is more precise than the older block-global texture map.
    pub fn element_face(&self, state: &State, face: Face, index: usize) -> Option<FaceSpec> {
        self.element_face_at(state, face, index, (0, 0, 0))
    }
    pub fn element_face_at(
        &self,
        state: &State,
        face: Face,
        index: usize,
        pos: (i32, i32, i32),
    ) -> Option<FaceSpec> {
        let (model, element) = self.chosen_element_at(state, index, pos)?;
        // A RenderFace stores the *local model face*; its vertices and
        // normals were rotated later by Part::point(). Inverse rotation
        // here would apply the same transform twice.
        let model_face = face;
        let uvlock = if model.uvlock {
            uvlock_permutation(face, model.x, model.y)?
        } else {
            [0, 1, 2, 3]
        };
        let model_key = model.key;
        let mut current = Some(model_key.clone());
        let mut visited = BTreeSet::new();
        let mut textures: Vec<&serde_json::Map<String, Value>> = Vec::new();
        let mut model_elements = None;
        while let Some(key) = current {
            if visited.len() >= 16 || !visited.insert(key.clone()) {
                return None;
            }
            let Some(doc) = self.files.get(&key) else {
                break;
            };
            if let Some(map) = doc.get("textures").and_then(Value::as_object) {
                textures.push(map);
            }
            if model_elements.is_none() {
                model_elements = doc.get("elements").and_then(Value::as_array);
            }
            let source_ns = key.strip_prefix("model:")?.split_once(':')?.0;
            current = doc
                .get("parent")
                .and_then(Value::as_str)
                .and_then(|s| normalize_model(s, source_ns));
        }
        let _ = model_elements; // validated inheritance, chosen element is exact
        let name = match model_face {
            Face::Top => "up",
            Face::Bottom => "down",
            Face::PosX => "east",
            Face::NegX => "west",
            Face::PosZ => "south",
            Face::NegZ => "north",
        };
        let face_doc = element.get("faces")?.get(name)?;
        let mut raw = face_doc.get("texture")?.as_str()?.to_string();
        let mut visited_aliases = BTreeSet::new();
        while let Some(alias) = raw.strip_prefix('#') {
            if visited_aliases.len() >= 16 || !visited_aliases.insert(alias.to_string()) {
                return None;
            }
            raw = textures
                .iter()
                .find_map(|m| m.get(alias).and_then(Value::as_str))?
                .into();
        }
        let default_ns = model_key.strip_prefix("model:")?.split_once(':')?.0;
        let texture = normalize_texture(&raw, default_ns)?;
        let rotation = match face_doc.get("rotation") {
            None => 0,
            Some(v) => u16::try_from(v.as_u64()?).ok()?,
        };
        if rotation % 90 != 0 || rotation >= 360 {
            return None;
        }
        let uv = if let Some(v) = face_doc.get("uv") {
            let a = v.as_array()?;
            if a.len() != 4 {
                return None;
            }
            let mut out = [0f32; 4];
            for (i, item) in a.iter().enumerate() {
                let value = item.as_f64()? as f32;
                if !value.is_finite() || !(-256.0..=256.0).contains(&value) {
                    return None;
                }
                out[i] = value;
            }
            out
        } else {
            implicit_face_uv(element, model_face)?
        };
        let tint_index = face_doc
            .get("tintindex")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok());
        Some(FaceSpec {
            texture,
            uv,
            rotation,
            tint_index,
            uvlock,
        })
    }
    pub fn texture(&self, state: &State, face: Face) -> Option<String> {
        self.texture_at(state, face, (0, 0, 0))
    }
    pub fn texture_at(&self, state: &State, face: Face, pos: (i32, i32, i32)) -> Option<String> {
        let chosen = self.selections_at(state, pos)?.into_iter().next()?;
        let lookup_face = face.undo_variant_rotation(chosen.x, chosen.y);
        let model_key = chosen.key;
        let fallback_ns = model_key
            .strip_prefix("model:")?
            .split_once(':')?
            .0
            .to_string();
        let mut maps = Vec::new();
        let mut visited = BTreeSet::new();
        let mut current = Some(model_key.clone());
        while let Some(key) = current {
            if visited.len() >= 16 || !visited.insert(key.clone()) {
                return None;
            }
            let Some(model) = self.files.get(&key) else {
                break;
            };
            maps.push(model.get("textures").and_then(Value::as_object));
            let source_ns = key.strip_prefix("model:")?.split_once(':')?.0;
            current = model
                .get("parent")
                .and_then(Value::as_str)
                .and_then(|v| normalize_model(v, source_ns));
        }
        // Newer models override inherited parent texture keys.
        let get = |key: &str| -> Option<String> {
            for map in &maps {
                if let Some(value) = map.and_then(|m| m.get(key)).and_then(Value::as_str) {
                    return Some(value.to_string());
                }
            }
            None
        };
        for key in face_keys(lookup_face, &model_key) {
            let Some(mut texture) = get(key) else {
                continue;
            };
            let mut seen = BTreeSet::new();
            for _ in 0..16 {
                if !texture.starts_with('#') {
                    break;
                }
                if !seen.insert(texture.clone()) {
                    return None;
                }
                texture = get(texture.trim_start_matches('#'))?;
            }
            if texture.starts_with('#') {
                return None;
            }
            if let Some(id) = normalize_texture(&texture, &fallback_ns) {
                return Some(id);
            }
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn s(name: &str, props: &[(&str, &str)]) -> State {
        State {
            name: name.into(),
            properties: props
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }
    #[test]
    fn vanilla_tintindex_is_preserved_only_on_explicit_tinted_faces() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/minecraft/blockstates/grass_block.json",
            br#"{"variants":{"":{"model":"minecraft:block/grass_block"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/minecraft/models/block/grass_block.json",
            br##"{"textures":{"top":"minecraft:block/grass_block_top",
                "side":"minecraft:block/grass_block_side"},
                "elements":[{"from":[0,0,0],"to":[16,16,16],
                  "faces":{"up":{"texture":"#top","tintindex":0},
                  "north":{"texture":"#side"}}}]}"##,
        )
        .unwrap();
        let s = s("minecraft:grass_block", &[]);
        assert_eq!(
            bank.element_face_at(&s, Face::Top, 0, (0, 64, 0))
                .unwrap()
                .tint_index,
            Some(0)
        );
        assert_eq!(
            bank.element_face_at(&s, Face::NegZ, 0, (0, 64, 0))
                .unwrap()
                .tint_index,
            None
        );
    }
    #[test]
    fn weighted_variants_are_stable_per_source_xyz_and_match_texture_geometry() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/ore.json",
            br#"{"variants":{"":[
               {"model":"test:block/short","weight":1},
               {"model":"test:block/tall","weight":3}]}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/short.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,8,16],
                 "faces":{"up":{"texture":"test:block/short","uv":[0,0,16,16]}}}]}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/tall.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                 "faces":{"up":{"texture":"test:block/tall","uv":[0,0,16,16]}}}]}"#,
        )
        .unwrap();
        let state = s("test:ore", &[]);
        let mut short = 0;
        let mut tall = 0;
        for x in -64..64 {
            let pos = (x, 64, -14);
            let a = bank.parts_at(&state, pos).unwrap();
            let b = bank.parts_at(&state, pos).unwrap();
            assert_eq!(
                a.iter().map(|p| p.bounds).collect::<Vec<_>>(),
                b.iter().map(|p| p.bounds).collect::<Vec<_>>()
            );
            let texture = bank
                .element_face_at(&state, Face::Top, 0, pos)
                .unwrap()
                .texture;
            let key = bank.weighted_selection_key(&state, pos);
            assert_eq!(key, bank.weighted_selection_key(&state, pos));
            if a[0].bounds.max[1] < 0.5 {
                short += 1;
                assert_eq!(texture, "test:short");
            } else {
                tall += 1;
                assert_eq!(texture, "test:tall");
            }
        }
        assert!(short > 0 && tall > short);
    }
    #[test]
    fn multipart_weighted_apply_uses_independent_stable_choices() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/garden.json",
            br#"{"multipart":[
                {"apply":[{"model":"test:block/short","weight":1},
                          {"model":"test:block/tall","weight":1}]},
                {"when":{"side":"north"},
                 "apply":[{"model":"test:block/tall","weight":1},
                          {"model":"test:block/short","weight":1}]}]}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/short.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,8,16],
                 "faces":{"up":{"texture":"test:block/short"}}}]}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/tall.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                 "faces":{"up":{"texture":"test:block/tall"}}}]}"#,
        )
        .unwrap();
        let state = s("test:garden", &[("side", "north")]);
        let source = (8, 70, -3);
        let chosen = bank.selections_at(&state, source).unwrap();
        assert_eq!(chosen.len(), 2);
        assert_eq!(
            chosen.iter().map(|m| m.key.clone()).collect::<Vec<_>>(),
            bank.selections_at(&state, source)
                .unwrap()
                .iter()
                .map(|m| m.key.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(bank.parts_at(&state, source).unwrap().len(), 2);
        assert!(choose_weighted(
            &serde_json::json!([
                {"model":"x","weight":0}
            ]),
            0
        )
        .is_none());
    }
    #[test]
    fn state_variants_inherit_named_top_side_and_preserve_namespace() {
        let mut bank = ModelBank::default();
        bank.insert("assets/test/blockstates/widget.json",
            br#"{"variants":{"facing=north":{"model":"test:block/widget_n"},"":{"model":"test:block/widget"}}}"#).unwrap();
        bank.insert("assets/test/models/block/widget_n.json",
            br##"{"parent":"test:block/cube_column","textures":{"end":"test:block/widget_end","side":"#body","body":"test:block/widget_wall"}}"##).unwrap();
        let state = s("test:widget", &[("facing", "north")]);
        assert_eq!(
            bank.texture(&state, Face::Top),
            Some("test:widget_end".into())
        );
        assert_eq!(
            bank.texture(&state, Face::PosZ),
            Some("test:widget_wall".into())
        );
        assert!(bank.texture(&s("other:widget", &[]), Face::Top).is_none());
    }
    #[test]
    fn cyclic_texture_aliases_and_untrusted_paths_fail_closed() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/minecraft/blockstates/loop.json",
            br#"{"variants":{"":{"model":"minecraft:block/loop"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/minecraft/models/block/loop.json",
            br##"{"textures":{"all":"#one","one":"#all"}}"##,
        )
        .unwrap();
        assert_eq!(bank.texture(&s("minecraft:loop", &[]), Face::Top), None);
        // Non-block model families are valid parents for source blocks.
        assert!(bank
            .insert("assets/minecraft/models/item/bad.json", b"{}")
            .unwrap());
        assert!(!bank
            .insert("assets/../models/block/thing.json", b"{}")
            .unwrap());
    }
    #[test]
    fn vanilla_model_elements_produce_exact_unrotated_cuboids() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/window.json",
            br#"{"variants":{"":{"model":"test:block/window"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/window.json",
            br#"{"elements":[{"from":[3,1,4],"to":[13,15,12]}]}"#,
        )
        .unwrap();
        let elements = bank.elements(&s("test:window", &[])).unwrap();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].min, [3.0 / 16.0 - 0.5, 1.0 / 16.0 - 0.5, -0.25]);
        assert_eq!(
            elements[0].max,
            [13.0 / 16.0 - 0.5, 15.0 / 16.0 - 0.5, 0.25]
        );
    }
    #[test]
    fn rotated_or_multipart_models_do_not_invent_geometry() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/rot.json",
            br#"{"variants":{"":{"model":"test:block/rot","x":90}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/rot.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16]}]}"#,
        )
        .unwrap();
        assert_eq!(
            bank.elements(&s("test:rot", &[])).unwrap(),
            vec![Box3::FULL]
        );
    }
    #[test]
    fn quarter_turn_variant_rotates_exact_cuboid_and_face_texture() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/lever.json",
            br#"{"variants":{"":{"model":"test:block/lever","y":90}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/lever.json",
            br#"{"textures":{"north":"test:block/north","east":"test:block/east"},
                "elements":[{"from":[0,0,0],"to":[4,12,16]}]}"#,
        )
        .unwrap();
        let s = s("test:lever", &[]);
        let a = bank.elements(&s).unwrap();
        assert_eq!(a[0].max[2], -0.25);
        assert_eq!(bank.texture(&s, Face::PosX), Some("test:north".into()));
        assert_eq!(bank.texture(&s, Face::PosZ), Some("test:east".into()));
    }
    #[test]
    fn unsupported_variant_rotation_fail_closed() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/bad.json",
            br#"{"variants":{"":{"model":"test:block/bad","y":45}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/bad.json",
            br#"{"elements":[{"from":[0,0,0],"to":[16,16,16]}]}"#,
        )
        .unwrap();
        assert!(bank.elements(&s("test:bad", &[])).is_none());
        assert_eq!(bank.texture(&s("test:bad", &[]), Face::Top), None);
    }
    #[test]
    fn uvlock_corners_preserve_world_texture_basis_with_any_variant_turn() {
        for x in [0, 90, 180, 270] {
            for y in [0, 90, 180, 270] {
                for face in [
                    Face::Top,
                    Face::Bottom,
                    Face::PosX,
                    Face::NegX,
                    Face::PosZ,
                    Face::NegZ,
                ] {
                    let permutation = uvlock_permutation(face, x, y).unwrap();
                    let mut sorted = permutation;
                    sorted.sort_unstable();
                    assert_eq!(sorted, [0, 1, 2, 3]);
                }
            }
        }
        assert_eq!(uvlock_permutation(Face::Top, 0, 0), Some([0, 1, 2, 3]));
        assert_ne!(uvlock_permutation(Face::Top, 0, 90), Some([0, 1, 2, 3]));
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/turn.json",
            br#"{"variants":{"":{"model":"test:block/turn","y":90,"uvlock":true}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/turn.json",
            br##"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                "faces":{"up":{"texture":"test:block/checker"}}}]}"##,
        )
        .unwrap();
        let st = s("test:turn", &[]);
        assert_eq!(bank.parts(&st).unwrap().len(), 1);
        assert_ne!(
            bank.element_face(&st, Face::Top, 0).unwrap().uvlock,
            [0, 1, 2, 3]
        );
    }
    #[test]
    fn json_cullface_is_optional_and_y_variant_rotates_world_direction() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/rotor.json",
            br#"{"variants":{"":{"model":"test:block/rotor","y":90}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/rotor.json",
            br##"{"elements":[{"from":[0,0,0],"to":[16,16,16],
                "faces":{"east":{"texture":"test:block/a","cullface":"east"},
                         "up":{"texture":"test:block/b"}}}]}"##,
        )
        .unwrap();
        let state = s("test:rotor", &[]);
        assert_eq!(
            bank.face_cull_direction(&state, Face::PosX, 0),
            Some(Some(Face::PosZ))
        );
        assert_eq!(bank.face_cull_direction(&state, Face::Top, 0), Some(None));
        assert_eq!(bank.face_cull_direction(&state, Face::NegX, 0), None);
        assert_eq!(bank.face_cull_direction(&state, Face::Top, 1), None);
    }
    #[test]
    fn explicit_json_model_only_renders_declared_faces_with_original_local_uvs() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/rotator.json",
            br#"{"variants":{"":{"model":"test:block/rotator","y":90}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/rotator.json",
            br##"{"textures":{"end":"test:block/red"},"elements":[
                {"from":[0,0,0],"to":[8,16,16],
                 "faces":{"east":{"texture":"#end","uv":[1,2,8,14]}}}]}"##,
        )
        .unwrap();
        let state = s("test:rotator", &[]);
        assert_eq!(bank.face_defined(&state, Face::PosX, 0), Some(true));
        assert_eq!(bank.face_defined(&state, Face::NegZ, 0), Some(false));
        assert_eq!(bank.face_defined(&state, Face::Top, 0), Some(false));
        let selected = bank.element_face(&state, Face::PosX, 0).unwrap();
        assert_eq!(selected.uv, [1.0, 2.0, 8.0, 14.0]);
        assert_eq!(selected.texture, "test:red");
    }
    #[test]
    fn absent_vanilla_cube_parent_recovers_geometry_and_face_culling() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/bricks.json",
            br#"{"variants":{"":{"model":"test:block/bricks"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/bricks.json",
            br##"{"parent":"minecraft:block/cube_all",
                "textures":{"all":"test:block/terracotta"}}"##,
        )
        .unwrap();
        let st = s("test:bricks", &[]);
        assert_eq!(bank.parts(&st).unwrap().len(), 1);
        assert_eq!(bank.parts(&st).unwrap()[0].bounds, Box3::FULL);
        for face in [
            Face::Top,
            Face::Bottom,
            Face::NegX,
            Face::PosX,
            Face::NegZ,
            Face::PosZ,
        ] {
            assert_eq!(bank.face_defined(&st, face, 0), Some(true));
            assert_eq!(bank.face_cull_direction(&st, face, 0), Some(Some(face)));
            assert_eq!(
                bank.element_face(&st, face, 0).unwrap().texture,
                "test:terracotta"
            );
        }
        assert!(vanilla_parent_elements("model:test:cube_all").is_none());
    }
    #[test]
    fn vanilla_cube_column_parent_preserves_end_side_texture_aliases() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/log.json",
            br#"{"variants":{"":{"model":"test:block/log"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/log.json",
            br##"{"parent":"minecraft:block/cube_column",
                "textures":{"end":"test:block/cap","side":"test:block/bark"}}"##,
        )
        .unwrap();
        let st = s("test:log", &[]);
        assert_eq!(
            bank.element_face(&st, Face::Top, 0).unwrap().texture,
            "test:cap"
        );
        assert_eq!(
            bank.element_face(&st, Face::NegZ, 0).unwrap().texture,
            "test:bark"
        );
    }
    #[test]
    fn vanilla_cross_parent_keeps_two_zero_thickness_quads() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/flower.json",
            br#"{"variants":{"":{"model":"test:block/flower"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/flower.json",
            br##"{"parent":"minecraft:block/cross",
                "textures":{"cross":"test:block/flower_png"}}"##,
        )
        .unwrap();
        let st = s("test:flower", &[]);
        let parts = bank.parts(&st).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].bounds.min[2], parts[0].bounds.max[2]);
        assert!(parts.iter().all(|p| p.rotation.is_some()));
        assert_eq!(bank.face_defined(&st, Face::Top, 0), Some(false));
        assert_eq!(bank.face_defined(&st, Face::NegZ, 0), Some(true));
        assert_eq!(
            bank.element_face(&st, Face::NegZ, 0).unwrap().uv,
            [0.0, 0.0, 16.0, 16.0]
        );
        assert_eq!(
            bank.element_face(&st, Face::NegZ, 1).unwrap().texture,
            "test:flower_png"
        );
    }
    #[test]
    fn nested_modded_item_models_and_custom_parent_support_extended_exact_geometry() {
        let mut bank = ModelBank::default();
        bank.insert("assets/create.addon/blockstates/gizmo.json",
            br#"{"variants":{"":{"model":"create.addon:item/machine/gizmo"}}}"#).unwrap();
        bank.insert("assets/create.addon/models/item/machine/gizmo.json",
            br##"{"parent":"create.addon:custom/complex/base","textures":{
                "all":"create.addon:item/machine/gizmo",
                "side":"create.addon:entity/machine/side"}}"##).unwrap();
        bank.insert("assets/create.addon/models/custom/complex/base.json",
            br##"{"elements":[{"from":[-16,0,-8],"to":[32,16,24],"faces":{
                "north":{"texture":"#side","uv":[0,0,16,16]},
                "up":{"texture":"#all","uv":[0,0,16,16]}}}]}"##).unwrap();
        let state = s("create.addon:gizmo", &[]);
        let parts = bank.parts_at(&state, (34, 66, -42)).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].bounds.min, [-1.5, -0.5, -1.0]);
        assert_eq!(parts[0].bounds.max, [1.5, 0.5, 1.0]);
        assert_eq!(
            bank.element_face_at(&state, Face::NegZ, 0, (34, 66, -42)).unwrap().texture,
            "create.addon:entity/machine/side"
        );
        assert_eq!(
            bank.element_face_at(&state, Face::Top, 0, (34, 66, -42)).unwrap().texture,
            "create.addon:item/machine/gizmo"
        );
        for bad in [
            "assets/create.addon/models/item/../escape.json",
            "assets/create.addon/models/UPPER.json",
            "assets/create.addon/models/item//empty.json",
        ] {
            assert!(!bank.insert(bad, b"{}").unwrap(), "{bad}");
        }
    }
    #[test]
    fn bounded_large_block_models_render_all_selected_elements_consistently() {
        let mut bank = ModelBank::default();
        bank.insert("assets/example/blockstates/engine.json",
            br#"{"variants":{"":{"model":"example:block/engine"}}}"#).unwrap();
        let one = serde_json::json!({"from":[0,0,0],"to":[16,16,16],
            "faces":{"north":{"texture":"#all"}}});
        let model = serde_json::json!({"textures":{"all":"example:block/engine"},
            "elements":vec![one.clone(); 160]});
        bank.insert("assets/example/models/block/engine.json",
            &serde_json::to_vec(&model).unwrap()).unwrap();
        let state = s("example:engine", &[]);
        assert_eq!(bank.parts_at(&state, (1, 2, 3)).unwrap().len(), 160);
        assert_eq!(
            bank.element_face_at(&state, Face::NegZ, 159, (1, 2, 3)).unwrap().texture,
            "example:engine"
        );
        assert_eq!(bank.face_defined_at(&state, Face::Top, 159, (1, 2, 3)), Some(false));
        let too_many = serde_json::json!({"textures":{"all":"example:block/engine"},
            "elements":vec![one; 257]});
        bank.insert("assets/example/models/block/engine.json",
            &serde_json::to_vec(&too_many).unwrap()).unwrap();
        assert!(bank.parts_at(&state, (1, 2, 3)).is_none());
    }
    #[test]
    fn dotted_mod_resource_json_resolves_texture_and_keeps_namespace() {
        let mut bank = ModelBank::default();
        assert_eq!(
            bank.insert(
                "assets/create.addon/blockstates/machine/gearbox.v2.json",
                br#"{"variants":{"":{"model":"create.addon:block/machine/gearbox.v2"}}}"#,
            ),
            Ok(true)
        );
        assert_eq!(
            bank.insert(
                "assets/create.addon/models/block/machine/gearbox.v2.json",
                br##"{"parent":"minecraft:block/cube_all",
                     "textures":{"all":"create.addon:block/machine/casing.v2"}}"##,
            ),
            Ok(true)
        );
        let state = s("create.addon:machine/gearbox.v2", &[]);
        assert_eq!(
            bank.element_face(&state, Face::Top, 0).unwrap().texture,
            "create.addon:machine/casing.v2"
        );
        assert_eq!(
            bank.insert("assets/create.addon/models/block/../secret.json", b"{}"),
            Ok(false)
        );
        assert_eq!(normalize_texture("create.addon:block/../unsafe", "minecraft"), None);
        assert_eq!(normalize_model("create.addon:block/a//b", "minecraft"), None);
    }
    #[test]
    fn model_bank_accepts_thousands_of_original_resource_models() {
        let mut bank = ModelBank::default();
        for i in 0..4_200 {
            let path = format!("assets/test/models/block/model_{i}.json");
            assert_eq!(bank.insert(&path, b"{}"), Ok(true));
        }
        assert_eq!(bank.count(), 4_200);
        let old = bank.total_json_bytes;
        assert_eq!(
            bank.insert("assets/test/models/block/model_0.json", b"{}"),
            Ok(true)
        );
        assert_eq!(bank.total_json_bytes, old);
    }
    #[test]
    fn vanilla_implicit_uv_preserves_partial_element_face_dimensions() {
        let slab: Value = serde_json::from_str(r#"{"from":[2,0,3],"to":[14,8,13]}"#).unwrap();
        assert_eq!(
            implicit_face_uv(&slab, Face::Top),
            Some([2.0, 3.0, 14.0, 13.0])
        );
        assert_eq!(
            implicit_face_uv(&slab, Face::Bottom),
            Some([2.0, 3.0, 14.0, 13.0])
        );
        assert_eq!(
            implicit_face_uv(&slab, Face::NegZ),
            Some([2.0, 8.0, 14.0, 16.0])
        );
        assert_eq!(
            implicit_face_uv(&slab, Face::PosZ),
            Some([2.0, 8.0, 14.0, 16.0])
        );
        assert_eq!(
            implicit_face_uv(&slab, Face::NegX),
            Some([3.0, 8.0, 13.0, 16.0])
        );
        assert_eq!(
            implicit_face_uv(&slab, Face::PosX),
            Some([3.0, 8.0, 13.0, 16.0])
        );
    }
    #[test]
    fn inherited_model_missing_uv_uses_element_bounds_not_full_texture() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/platform.json",
            br#"{"variants":{"":{"model":"test:block/platform"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/platform.json",
            br##"{"textures":{"top":"test:block/wood"},"elements":[
                {"from":[0,0,0],"to":[16,8,16],
                 "faces":{"north":{"texture":"#top"}}}]}"##,
        )
        .unwrap();
        let f = bank
            .element_face(&s("test:platform", &[]), Face::NegZ, 0)
            .unwrap();
        assert_eq!(f.uv, [0.0, 8.0, 16.0, 16.0]);
        assert_eq!(f.texture, "test:wood");
    }
    #[test]
    fn per_element_face_texture_alias_crop_and_quarter_turn() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/machine.json",
            br#"{"variants":{"":{"model":"test:block/machine"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/machine.json",
            br##"{"textures":{"surface":"test:block/casing"},
              "elements":[{"from":[0,0,0],"to":[16,16,16],
                "faces":{"north":{"texture":"#surface","uv":[4,2,12,14],"rotation":90},
                         "up":{"texture":"test:block/top"}}}]}"##,
        )
        .unwrap();
        let state = s("test:machine", &[]);
        assert_eq!(
            bank.element_face(&state, Face::NegZ, 0),
            Some(FaceSpec {
                texture: "test:casing".into(),
                uv: [4.0, 2.0, 12.0, 14.0],
                rotation: 90,
                uvlock: [0, 1, 2, 3],
                tint_index: None,
            })
        );
        assert_eq!(
            bank.element_face(&state, Face::Top, 0).unwrap().texture,
            "test:top"
        );
        assert!(bank.element_face(&state, Face::PosX, 0).is_none());
        assert!(bank.element_face(&state, Face::NegZ, 1).is_none());
    }
    #[test]
    fn multipart_conditions_flatten_matching_models_and_keep_element_faces() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/gate.json",
            br#"{"multipart":[
                {"apply":{"model":"test:block/post"}},
                {"when":{"north":"true|auto"},"apply":{"model":"test:block/north"}},
                {"when":{"OR":[{"east":"true"},{"west":"true"}]},
                 "apply":{"model":"test:block/east"}}]}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/post.json",
            br##"{"textures":{"all":"test:block/post"},"elements":[
                {"from":[7,0,7],"to":[9,16,9],
                 "faces":{"up":{"texture":"#all"}}}]}"##,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/north.json",
            br##"{"textures":{"all":"test:block/north"},"elements":[
                {"from":[7,6,0],"to":[9,9,7],
                 "faces":{"north":{"texture":"#all"}}}]}"##,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/east.json",
            br##"{"textures":{"all":"test:block/east"},"elements":[
                {"from":[9,6,7],"to":[16,9,9],
                 "faces":{"east":{"texture":"#all"}}}]}"##,
        )
        .unwrap();
        let state = s("test:gate", &[("north", "true"), ("east", "false")]);
        assert_eq!(bank.elements(&state).unwrap().len(), 2);
        assert_eq!(
            bank.element_face(&state, Face::Top, 0).unwrap().texture,
            "test:post"
        );
        assert_eq!(
            bank.element_face(&state, Face::NegZ, 1).unwrap().texture,
            "test:north"
        );
        let all = s("test:gate", &[("north", "true"), ("east", "true")]);
        assert_eq!(bank.elements(&all).unwrap().len(), 3);
        let none = s("test:gate", &[]);
        assert_eq!(bank.elements(&none).unwrap().len(), 1);
    }
    #[test]
    fn nested_multipart_condition_semantics_fail_closed_for_invalid_data() {
        let state = s("test:machine", &[("on", "true"), ("axis", "z")]);
        let good: Value = serde_json::from_str(
            r#"{"AND":[{"on":"false|true"},{"OR":[{"axis":"x"},{"axis":"z"}]}]}"#,
        )
        .unwrap();
        assert_eq!(condition_matches(&good, &state, 0), Some(true));
        let bad: Value = serde_json::from_str(r#"{"OR":[]}"#).unwrap();
        assert_eq!(condition_matches(&bad, &state, 0), None);
        let missing: Value = serde_json::from_str(r#"{"axis":"x"}"#).unwrap();
        assert_eq!(condition_matches(&missing, &state, 0), Some(false));
    }
    #[test]
    fn vanilla_crossed_plant_planes_keep_opposing_faces_and_no_shade() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/minecraft/blockstates/oak_sapling.json",
            br#"{"variants":{"":{"model":"minecraft:block/oak_sapling"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/minecraft/models/block/oak_sapling.json",
            br##"{"parent":"minecraft:block/cross",
                "textures":{"cross":"minecraft:block/oak_sapling"}}"##,
        )
        .unwrap();
        bank.insert(
            "assets/minecraft/models/block/cross.json",
            br##"{"ambientocclusion":false,
              "textures":{"particle":"#cross"},
              "elements":[
                {"from":[0.8,0,8],"to":[15.2,16,8],
                 "rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},
                 "shade":false,
                 "faces":{"north":{"uv":[0,0,16,16],"texture":"#cross"},
                          "south":{"uv":[0,0,16,16],"texture":"#cross"}}},
                {"from":[8,0,0.8],"to":[8,16,15.2],
                 "rotation":{"origin":[8,8,8],"axis":"y","angle":45,"rescale":true},
                 "shade":false,
                 "faces":{"west":{"uv":[0,0,16,16],"texture":"#cross"},
                          "east":{"uv":[0,0,16,16],"texture":"#cross"}}}
              ]}"##,
        )
        .unwrap();
        let state = s("minecraft:oak_sapling", &[]);
        let parts = bank.parts(&state).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].rotation.is_some());
        assert_eq!(parts[0].bounds.min[2], parts[0].bounds.max[2]);
        assert_eq!(parts[1].bounds.min[0], parts[1].bounds.max[0]);
        assert_eq!(bank.face_defined(&state, Face::Top, 0), Some(false));
        assert_eq!(bank.face_defined(&state, Face::NegZ, 0), Some(true));
        assert_eq!(bank.face_defined(&state, Face::PosX, 1), Some(true));
        assert_eq!(bank.element_unshaded(&state, 0), Some(true));
        assert_eq!(bank.element_unshaded(&state, 1), Some(true));
        assert_eq!(
            bank.element_face(&state, Face::NegZ, 0).unwrap().texture,
            "minecraft:oak_sapling"
        );
    }
    #[test]
    fn flat_plane_without_declared_normal_face_is_not_invented() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/invalid.json",
            br#"{"variants":{"":{"model":"test:block/invalid"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/invalid.json",
            br#"{"elements":[{"from":[1,0,8],"to":[15,16,8],
                "faces":{"up":{"texture":"test:block/leaf"}}}]}"#,
        )
        .unwrap();
        assert!(bank.parts(&s("test:invalid", &[])).is_none());
    }
    #[test]
    fn tilted_resource_model_parts_preserve_true_vertices() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/lever.json",
            br#"{"variants":{"":{"model":"test:block/handle"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/test/models/block/handle.json",
            br#"{"elements":[{"from":[7,1,7],"to":[9,14,9],
               "rotation":{"axis":"z","origin":[8,8,8],"angle":22.5}}]}"#,
        )
        .unwrap();
        let state = s("test:lever", &[]);
        let parts = bank.parts(&state).unwrap();
        assert_eq!(parts.len(), 1);
        assert!(parts[0].rotation.is_some());
        assert!(!parts[0].boundary(1, true));
        assert!(bank.elements(&state).is_none()); // legacy AABB route refuses
    }
    #[test]
    fn inherited_tinted_cross_and_leaves_preserve_biome_tintindex() {
        let mut bank = ModelBank::default();
        for (name, parent, key) in [
            ("fern", "tinted_cross", "cross"),
            ("oak_leaves", "leaves", "all"),
            ("flower", "cross", "cross"),
        ] {
            bank.insert(
                &format!("assets/minecraft/blockstates/{name}.json"),
                format!(r#"{{"variants":{{"":{{"model":"minecraft:block/{name}"}}}}}}"#).as_bytes(),
            )
            .unwrap();
            bank.insert(
                &format!("assets/minecraft/models/block/{name}.json"),
                format!(r##"{{"parent":"minecraft:block/{parent}","textures":{{"{key}":"minecraft:block/{name}"}}}}"##).as_bytes()
            ).unwrap();
        }
        let fern = s("minecraft:fern", &[]);
        let fern_parts = bank.parts(&fern).unwrap();
        assert_eq!(fern_parts.len(), 2);
        assert!(fern_parts.iter().all(|p| p.rotation.is_some()));
        assert_eq!(
            bank.element_face(&fern, Face::NegZ, 0).unwrap().tint_index,
            Some(0)
        );
        let leaves = s("minecraft:oak_leaves", &[]);
        assert_eq!(bank.parts(&leaves).unwrap().len(), 1);
        for face in [
            Face::Top,
            Face::Bottom,
            Face::PosX,
            Face::NegX,
            Face::PosZ,
            Face::NegZ,
        ] {
            assert_eq!(
                bank.element_face(&leaves, face, 0).unwrap().tint_index,
                Some(0)
            );
            assert_eq!(bank.face_cull_direction(&leaves, face, 0), Some(Some(face)));
        }
        assert_eq!(
            bank.element_face(&s("minecraft:flower", &[]), Face::NegZ, 0)
                .unwrap()
                .tint_index,
            None
        );
    }
    #[test]
    fn vanilla_fence_parent_templates_recover_real_post_and_rails() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/minecraft/blockstates/oak_fence.json",
            br#"{"multipart":[{"apply":{"model":"minecraft:block/oak_fence_post"}},
            {"when":{"north":"true"},"apply":{"model":"minecraft:block/oak_fence_side"}}]}"#,
        )
        .unwrap();
        bank.insert("assets/minecraft/models/block/oak_fence_post.json",
            br##"{"parent":"minecraft:block/template_fence_post", "textures":{"texture":"minecraft:block/oak_planks"}}"##).unwrap();
        bank.insert("assets/minecraft/models/block/oak_fence_side.json",
            br##"{"parent":"minecraft:block/template_fence_side", "textures":{"texture":"minecraft:block/oak_planks"}}"##).unwrap();
        let disconnected = s("minecraft:oak_fence", &[("north", "false")]);
        let connected = s("minecraft:oak_fence", &[("north", "true")]);
        let post = bank.parts(&disconnected).unwrap();
        assert_eq!(post.len(), 1);
        assert_eq!(post[0].bounds.min, [-0.125, -0.5, -0.125]);
        let sections = bank.parts(&connected).unwrap();
        assert_eq!(sections.len(), 3);
        assert!((sections[1].bounds.max[1] - (-0.5 + 9.0 / 16.0)).abs() < 0.0001);
        assert_eq!(
            bank.element_face(&connected, Face::NegZ, 1)
                .unwrap()
                .texture,
            "minecraft:oak_planks"
        );
        assert_eq!(
            bank.element_face(&connected, Face::Top, 2)
                .unwrap()
                .tint_index,
            None
        );
        assert!(vanilla_parent_elements("model:other:template_fence_side").is_none());
    }
    #[test]
    fn fallback_cube_top_template_preserves_top_and_side_textures() {
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/minecraft/blockstates/example.json",
            br#"{"variants":{"":{"model":"minecraft:block/example"}}}"#,
        )
        .unwrap();
        bank.insert(
            "assets/minecraft/models/block/example.json",
            br##"{"parent":"minecraft:block/cube_top","textures":{
                "side":"minecraft:block/stone","top":"minecraft:block/grass_block_top"}}"##,
        )
        .unwrap();
        let state = s("minecraft:example", &[]);
        assert_eq!(
            bank.element_face(&state, Face::Top, 0).unwrap().texture,
            "minecraft:grass_block_top"
        );
        assert_eq!(
            bank.element_face(&state, Face::NegZ, 0).unwrap().texture,
            "minecraft:stone"
        );
        assert_eq!(
            bank.element_face(&state, Face::Bottom, 0).unwrap().texture,
            "minecraft:stone"
        );
    }
    #[test]
    fn texture_variants_match_all_state_properties_without_guessing() {
        assert!(variant_matches(
            "axis=y,facing=north",
            &s("x:block", &[("axis", "y"), ("facing", "north")])
        ));
        assert!(!variant_matches("axis=x", &s("x:block", &[("axis", "y")])));
        assert!(!variant_matches("unsupported", &s("x:block", &[])));
    }
}
