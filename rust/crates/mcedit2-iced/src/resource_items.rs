//! 物品显示上下文：实际父链、覆盖模型、显示变换和元素面。
use super::*;
use crate::root_transform::Affine;
#[derive(Clone, Debug)]
pub(crate) struct Quad {
    pub xyz: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub texture: String,
    pub tint: Option<u32>,
    pub shade: bool,
    pub colors: [[f32; 4]; 4],
    pub winding: f32,
    pub cull: bool,
    pub mode: Option<u32>,
}
#[derive(Clone, Debug)]
pub(crate) struct Scene {
    pub builtin: bool,
    pub quads: Vec<Quad>,
    pub layers: Vec<(String, u32)>,
    pub transform: Affine,
    pub flat: bool,
    pub gui_3d: bool,
    pub display_scale: [f32; 3],
}
fn vector(value: &Value) -> Result<[f32; 3], String> {
    let a = value
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("物品变换必须有三个坐标")?;
    let mut out = [0.; 3];
    for i in 0..3 {
        out[i] = a[i]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("物品坐标必须为有限数")? as f32;
    }
    Ok(out)
}
fn display(doc: Option<&Value>, left: bool) -> Result<(Affine, [f32; 3]), String> {
    let Some(doc) = doc else {
        return Ok((Affine::IDENTITY, [1.; 3]));
    };
    let mut rotation = doc
        .get("rotation")
        .map(vector)
        .transpose()?
        .unwrap_or([0.; 3]);
    let mut translation = doc
        .get("translation")
        .map(vector)
        .transpose()?
        .unwrap_or([0.; 3])
        .map(|v| (v / 16.).clamp(-5., 5.));
    let scale = doc
        .get("scale")
        .map(vector)
        .transpose()?
        .unwrap_or([1.; 3])
        .map(|v| v.clamp(-4., 4.));
    if left {
        rotation[1] = -rotation[1];
        rotation[2] = -rotation[2];
        translation[0] = -translation[0];
    }
    Ok((
        Affine::parse(
            &serde_json::json!({"rotation":rotation,"translation":translation,"scale":scale,"origin":"center"}),
        )?,
        scale,
    ))
}
fn texture(textures: &serde_json::Map<String, Value>, raw: &str) -> Result<String, String> {
    let mut raw = raw;
    let mut seen = BTreeSet::new();
    while let Some(key) = raw.strip_prefix('#') {
        if !seen.insert(key) {
            return Err("物品纹理别名循环".into());
        }
        raw = textures
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("缺少物品纹理别名 {key}"))?;
    }
    normalize_texture(raw).ok_or_else(|| format!("物品纹理 ID 无效：{raw}"))
}
impl ModelBank {
    pub(crate) fn item_scene(
        &self,
        id: &str,
        properties: &BTreeMap<String, f32>,
    ) -> Result<Scene, String> {
        self.item_scene_context(id, properties, "gui")
    }
    pub(crate) fn item_scene_context(
        &self,
        id: &str,
        properties: &BTreeMap<String, f32>,
        context: &str,
    ) -> Result<Scene, String> {
        let (ns, path) = id.split_once(':').ok_or("物品 ID 缺少命名空间")?;
        self.scene_model(format!("model:{ns}:item/{path}"), properties, context)
    }
    pub(crate) fn scene_model(
        &self,
        mut key: String,
        properties: &BTreeMap<String, f32>,
        context: &str,
    ) -> Result<Scene, String> {
        let mut overrides = BTreeSet::new();
        loop {
            if !overrides.insert(key.clone()) {
                return Err("物品覆盖模型循环".into());
            }
            let mut chain = Vec::new();
            let mut parents = BTreeSet::new();
            let mut current = key.clone();
            let mut generated = false;
            let mut builtin = false;
            loop {
                if !parents.insert(current.clone()) {
                    return Err("物品父模型循环".into());
                }
                let Some(doc) = self.files.get(&current) else {
                    if vanilla_parent_elements(&current).is_some() {
                        break;
                    }
                    return Err(format!("缺少物品模型 {current}"));
                };
                chain.push(doc);
                let Some(parent) = doc.get("parent") else {
                    break;
                };
                current = normalize_model(parent.as_str().ok_or("物品 parent 必须为字符串")?)
                    .ok_or("物品父模型 ID 无效")?;
                if current == "model:minecraft:builtin/generated" {
                    generated = true;
                    break;
                }
                if current == "model:minecraft:builtin/entity" {
                    builtin = true;
                    break;
                }
            }
            // 原版覆盖规则按数组倒序选择最后一个全部满足的候选。
            let mut selected = None;
            if let Some(list) = chain.iter().find_map(|d| d.get("overrides")) {
                for choice in list
                    .as_array()
                    .ok_or("物品 overrides 必须为数组")?
                    .iter()
                    .rev()
                {
                    let predicates = choice
                        .get("predicate")
                        .and_then(Value::as_object)
                        .ok_or("物品覆盖缺少 predicate")?;
                    let mut matches = true;
                    for (name, limit) in predicates {
                        let name = name.strip_prefix("minecraft:").unwrap_or(name);
                        let limit = limit
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .ok_or("物品 predicate 必须为有限数")?
                            as f32;
                        // 未提供的运行时属性不满足条件；不能编造玩家或世界上下文。
                        matches &= properties.get(name).is_some_and(|value| *value >= limit);
                    }
                    if matches {
                        selected = Some(
                            normalize_model(
                                choice
                                    .get("model")
                                    .and_then(Value::as_str)
                                    .ok_or("物品覆盖缺少 model")?,
                            )
                            .ok_or("覆盖模型 ID 无效")?,
                        );
                        break;
                    }
                }
            }
            if let Some(next) = selected.filter(|next| next != &key) {
                key = next;
                continue;
            }
            let right_context = match context {
                "thirdperson_lefthand" => Some("thirdperson_righthand"),
                "firstperson_lefthand" => Some("firstperson_righthand"),
                _ => None,
            };
            let (transform, display_scale) = display(
                chain.iter().find_map(|doc| {
                    let display = doc.get("display")?;
                    display
                        .get(context)
                        .or_else(|| right_context.and_then(|right| display.get(right)))
                }),
                right_context.is_some(),
            )?;
            let flat = match chain.iter().find_map(|d| d.get("gui_light")) {
                Some(Value::String(s)) if s == "front" => true,
                Some(Value::String(s)) if s == "side" => false,
                Some(_) => return Err("gui_light 必须为 front 或 side".into()),
                None => generated,
            };
            let mut scene = Scene {
                builtin,
                gui_3d: !generated,
                display_scale,
                quads: Vec::new(),
                layers: Vec::new(),
                transform,
                flat,
            };
            if builtin {
                return Ok(scene);
            }
            if generated {
                let mut textures = serde_json::Map::new();
                for doc in chain.iter().rev() {
                    if let Some(map) = doc.get("textures") {
                        textures.extend(map.as_object().ok_or("物品 textures 必须为对象")?.clone());
                    }
                }
                for layer in 0..5 {
                    let name = format!("layer{layer}");
                    if !textures.contains_key(&name) {
                        break;
                    }
                    scene
                        .layers
                        .push((texture(&textures, &format!("#{name}"))?, layer));
                }
                if scene.layers.is_empty() {
                    return Err("生成式物品模型没有 layer0".into());
                }
                return Ok(scene);
            }
            let compiled = graph::compile_item(
                &self.files,
                &key,
                context,
                &|spec| self.bake_obj(spec),
                &|key| self.layer_catalog(key),
            )?;
            append_model(&mut scene, compiled)?;
            return Ok(scene);
        }
    }
}

fn append_model(scene: &mut Scene, compiled: graph::Model) -> Result<(), String> {
    let mut compiled = compiled;
    for layer in &compiled.layers {
        compiled
            .triangles
            .extend(layer.instantiate(&crate::layer_renderer::Context {
                state: None,
                visual: None,
                params: None,
                catalog: None,
            })?);
    }
    for triangle in &compiled.triangles {
        scene.quads.push(Quad {
            xyz: [
                triangle.xyz[0],
                triangle.xyz[1],
                triangle.xyz[2],
                triangle.xyz[2],
            ],
            uv: [
                triangle.uv[0],
                triangle.uv[1],
                triangle.uv[2],
                triangle.uv[2],
            ],
            texture: triangle.texture.clone(),
            tint: triangle.tint_index,
            shade: true,
            colors: [0, 1, 2, 2]
                .map(|i| std::array::from_fn(|c| triangle.colors[i][c] * triangle.tint[c])),
            winding: 1.,
            cull: !triangle.double_sided,
            mode: Some(triangle.material_flags),
        });
    }
    for element in compiled.elements {
        let doc = &element.document;
        let from = vector(doc.get("from").ok_or("物品元素缺少 from")?)?;
        let to = vector(doc.get("to").ok_or("物品元素缺少 to")?)?;
        let bounds = Box3::from_model_units_directed(from, to)
            .or_else(|| Box3::from_model_units_plane(from, to))
            .ok_or("物品元素范围无效")?;
        let rotation = if let Some(rot) = doc.get("rotation") {
            let axis = match rot.get("axis").and_then(Value::as_str) {
                Some("x") => 0,
                Some("y") => 1,
                Some("z") => 2,
                _ => return Err("物品元素旋转轴无效".into()),
            };
            let angle = rot
                .get("angle")
                .and_then(Value::as_f64)
                .ok_or("物品元素旋转缺少 angle")? as f32;
            if ![-45., -22.5, 0., 22.5, 45.].contains(&angle) {
                return Err("物品元素角度无效".into());
            }
            Some(ElementRotation {
                axis,
                origin: read_rotation_origin(rot).ok_or("物品元素旋转原点无效")?,
                angle_degrees: angle,
                rescale: rot.get("rescale").and_then(Value::as_bool).unwrap_or(false),
            })
        } else {
            None
        };
        let part = Part {
            bounds,
            rotation,
            variant_x: 0,
            variant_y: 0,
            root: Some(element.root),
        };
        let faces = doc
            .get("faces")
            .and_then(Value::as_object)
            .ok_or("物品元素缺少 faces")?;
        for (name, face) in [
            ("up", Face::Top),
            ("down", Face::Bottom),
            ("east", Face::PosX),
            ("west", Face::NegX),
            ("south", Face::PosZ),
            ("north", Face::NegZ),
        ] {
            let Some(face_doc) = faces.get(name) else {
                continue;
            };
            let tex = texture(
                &element.textures,
                &face_texture_reference(
                    face_doc
                        .get("texture")
                        .and_then(Value::as_str)
                        .ok_or("物品面缺少 texture")?,
                ),
            )?;
            let uv = if let Some(uv) = face_doc.get("uv") {
                let a = uv
                    .as_array()
                    .filter(|a| a.len() == 4)
                    .ok_or("物品面 UV 必须有四个值")?;
                let mut result = [0.; 4];
                for i in 0..4 {
                    result[i] = a[i]
                        .as_f64()
                        .filter(|v| v.is_finite())
                        .ok_or("物品 UV 必须为有限数")? as f32;
                }
                result
            } else {
                implicit_face_uv(doc, face).ok_or("物品元素无法计算默认 UV")?
            };
            let rotation = face_doc
                .get("rotation")
                .map(|v| v.as_u64().ok_or("物品 UV rotation 无效"))
                .transpose()?
                .unwrap_or(0);
            if rotation >= 360 || rotation % 90 != 0 {
                return Err("物品 UV rotation 必须为四分之一转".into());
            }
            let mut corners = [
                [uv[0], uv[1]],
                [uv[2], uv[1]],
                [uv[2], uv[3]],
                [uv[0], uv[3]],
            ]
            .map(|p| p.map(|v| v / 16.));
            corners.rotate_right(rotation as usize / 90);
            let mut permutation = face.java_uv_indices();
            if element.root.determinant() < 0. {
                permutation.swap(1, 3);
            }
            let xyz = part.face_vertices(face.corners());
            scene.quads.push(Quad {
                xyz,
                uv: permutation.map(|i| corners[i as usize]),
                texture: tex,
                tint: face_doc
                    .get("tintindex")
                    .and_then(Value::as_u64)
                    .and_then(|i| u32::try_from(i).ok()),
                shade: doc.get("shade").and_then(Value::as_bool).unwrap_or(true),
                colors: [[1.; 4]; 4],
                winding: if crate::world_mesh::winding_reversed(&xyz, part.normal(face.normal())) {
                    -1.
                } else {
                    1.
                },
                cull: true,
                mode: element.render_type.map(|mode| mode.gpu_bits()),
            });
        }
    }

    Ok(())
}
pub(crate) fn append_elements(scene: &mut Scene, doc: Value) -> Result<(), String> {
    let render_type = doc
        .get("render_type")
        .map(|value| {
            crate::textures::AlphaMode::from_render_type(
                value.as_str().ok_or("内置元素 render_type 必须为字符串")?,
            )
        })
        .transpose()?;
    let textures = std::sync::Arc::new(
        doc.get("textures")
            .and_then(Value::as_object)
            .ok_or("内置元素模型缺少 textures")?
            .clone(),
    );
    let elements = doc
        .get("elements")
        .and_then(Value::as_array)
        .ok_or("内置模型缺少 elements")?
        .iter()
        .map(|document| graph::Element {
            document: document.clone(),
            textures: textures.clone(),
            root: Affine::IDENTITY,
            render_type,
        })
        .collect();
    append_model(
        scene,
        graph::Model {
            elements,
            ..Default::default()
        },
    )
}
