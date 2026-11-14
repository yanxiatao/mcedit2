//! 声明式模型层后端。资源包与插件共用数据合同，不执行 Java 渲染代码。
//! `$state` 读取属性，`$nbt` 读取类型化外观路径，map / scale / offset 显式转换。
use crate::{
    resource_models::{normalize_model, normalize_texture, ObjSelection},
    root_transform::Affine,
};
use mcedit2_world::{
    entity_catalog::EntityVisual,
    entity_model::{Catalog, PartPose},
    nbt_document::Value as Nbt,
    sections::State,
};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

pub(crate) struct Context<'a> {
    pub state: Option<&'a State>,
    pub visual: Option<&'a EntityVisual>,
    pub params: Option<&'a Map<String, Value>>,
    pub catalog: Option<&'a Catalog>,
}
impl<'a> Context<'a> {
    pub(crate) fn block(state: &'a State) -> Self {
        Self {
            state: Some(state),
            visual: None,
            params: None,
            catalog: None,
        }
    }
}
#[derive(Clone, Debug)]
enum ModelCatalog {
    Vanilla,
    Resource(Arc<Catalog>),
}
impl ModelCatalog {
    fn get(&self) -> Result<&Catalog, String> {
        match self {
            Self::Vanilla => Catalog::java_1_21_1().map_err(str::to_owned),
            Self::Resource(value) => Ok(value),
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Template {
    document: Arc<Value>,
    textures: Arc<Map<String, Value>>,
    catalog: ModelCatalog,
    root: Affine,
    state_only: bool,
    instances: Arc<Mutex<Instances>>,
}
#[derive(Debug, Default)]
struct Instances {
    entries: VecDeque<(State, Result<Arc<Vec<ObjSelection>>, String>, usize)>,
    bytes: usize,
}
fn state_only(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            !fields.contains_key("$nbt")
                && !fields.contains_key("$param")
                && fields.values().all(state_only)
        }
        Value::Array(items) => items.iter().all(state_only),
        _ => true,
    }
}
const RENDER_FIELDS: [&str; 9] = [
    "layer",
    "texture",
    "poses",
    "layer_transform",
    "visible",
    "parts",
    "material",
    "space",
    "part_transforms",
];
fn model_transform(value: &Value) -> Result<Affine, String> {
    if let Some(steps) = value
        .as_array()
        .filter(|steps| steps.first().is_some_and(Value::is_object))
    {
        steps.iter().try_fold(Affine::IDENTITY, |out, step| {
            out.compose(Affine::parse(step)?)
        })
    } else {
        Affine::parse(value)
    }
}

fn nbt_json(value: &Nbt, depth: usize) -> Result<Value, String> {
    if depth > 64 {
        return Err("模型 NBT 绑定超过 64 层".into());
    }
    Ok(match value {
        Nbt::Byte(v) => Value::from(*v),
        Nbt::Short(v) => Value::from(*v),
        Nbt::Int(v) => Value::from(*v),
        Nbt::Long(v) => Value::from(*v),
        Nbt::Float(v) => serde_json::Number::from_f64(f64::from(*v))
            .map(Value::Number)
            .ok_or("绑定 Float 非有限数")?,
        Nbt::Double(v) => serde_json::Number::from_f64(*v)
            .map(Value::Number)
            .ok_or("绑定 Double 非有限数")?,
        Nbt::String(v) => Value::String(v.clone()),
        Nbt::List(v) => Value::Array(
            v.items
                .iter()
                .map(|v| nbt_json(v, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        Nbt::ByteArray(v) => Value::Array(v.iter().map(|v| Value::from(*v)).collect()),
        Nbt::IntArray(v) => Value::Array(v.iter().map(|v| Value::from(*v)).collect()),
        Nbt::LongArray(v) => Value::Array(v.iter().map(|v| Value::from(*v)).collect()),
        Nbt::Compound(fields) => {
            let mut values = Map::new();
            for field in fields {
                if values
                    .insert(field.name.clone(), nbt_json(&field.value, depth + 1)?)
                    .is_some()
                {
                    return Err(format!("模型绑定 NBT Compound 重复字段 {}", field.name));
                }
            }
            Value::Object(values)
        }
    })
}
fn nbt_at(path: &Value, context: &Context<'_>) -> Result<Option<Value>, String> {
    let path = path
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("$nbt 路径不能为空")?;
    let root = path[0].as_str().ok_or("$nbt 路径必须从字段名开始")?;
    if root == "Rotation"
        && path.len() == 2
        && context
            .visual
            .is_some_and(|visual| visual.rotation.is_some())
    {
        let angles = context
            .visual
            .and_then(|v| v.rotation)
            .ok_or("模型绑定缺少源 Rotation")?;
        let axis = path[1]
            .as_u64()
            .filter(|v| *v < 2)
            .ok_or("Rotation 索引必须为 0 或 1")? as usize;
        return Ok(Some(Value::from(angles[axis])));
    }
    let Some(source) = context
        .visual
        .ok_or("模型绑定缺少源外观 NBT")?
        .source_field_optional(root)?
    else {
        return Ok(None);
    };
    let mut value = &source;
    for (at, key) in path[1..].iter().enumerate() {
        value = match (value, key) {
            (Nbt::Compound(fields), Value::String(key)) => {
                let mut found = fields.iter().filter(|f| f.name == *key);
                let Some(field) = found.next() else {
                    return Ok(None);
                };
                if found.next().is_some() {
                    return Err(format!("模型 NBT 字段重复：{key}"));
                }
                &field.value
            }
            (Nbt::List(list), Value::Number(index)) => {
                let index = index
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .ok_or("NBT 索引无效")?;
                let Some(value) = list.items.get(index) else {
                    return Ok(None);
                };
                value
            }
            (Nbt::ByteArray(_) | Nbt::IntArray(_) | Nbt::LongArray(_), Value::Number(index)) => {
                if at + 2 != path.len() {
                    return Err("NBT 数组元素不能继续沿字段路径访问".into());
                }
                let index = index
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .ok_or("NBT 数组索引无效")?;
                return Ok(match value {
                    Nbt::ByteArray(values) => values.get(index).map(|v| Value::from(*v)),
                    Nbt::IntArray(values) => values.get(index).map(|v| Value::from(*v)),
                    Nbt::LongArray(values) => values.get(index).map(|v| Value::from(*v)),
                    _ => return Err("NBT 数组来源类型改变".into()),
                });
            }
            _ => return Err("模型绑定 NBT 路径与源类型不匹配".into()),
        };
    }
    nbt_json(value, 0).map(Some)
}
fn evaluate(value: &Value, context: &Context<'_>, depth: usize) -> Result<Value, String> {
    if depth > 64 {
        return Err("模型参数嵌套超过 64 层".into());
    }
    match value {
        Value::Array(items) => items
            .iter()
            .map(|v| evaluate(v, context, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(fields)
            if fields.contains_key("$state")
                || fields.contains_key("$nbt")
                || fields.contains_key("$param") =>
        {
            if ["$state", "$nbt", "$param"]
                .into_iter()
                .filter(|key| fields.contains_key(*key))
                .count()
                != 1
            {
                return Err("同一模型绑定必须恰好声明一种来源".into());
            }
            if fields.keys().any(|k| {
                !matches!(
                    k.as_str(),
                    "$state" | "$nbt" | "$param" | "map" | "scale" | "offset" | "absent"
                )
            }) {
                return Err("模型绑定含未知字段".into());
            }
            if fields.contains_key("absent") && !fields.contains_key("$nbt") {
                return Err("absent 只能用于明确声明 NBT 字段缺失时的值".into());
            }
            let mut result = if let Some(key) = fields.get("$state") {
                let key = key.as_str().ok_or("$state 必须为属性名")?;
                let mut values = context
                    .state
                    .ok_or("缺少源方块状态")?
                    .properties
                    .iter()
                    .filter(|(name, _)| name == key);
                let value = values
                    .next()
                    .ok_or_else(|| format!("模型绑定缺少状态属性 {key}"))?;
                if values.next().is_some() {
                    return Err(format!("源状态属性重复：{key}"));
                }
                Value::String(value.1.clone())
            } else if let Some(key) = fields.get("$param") {
                let key = key.as_str().ok_or("$param 必须为参数名")?;
                context
                    .params
                    .and_then(|params| params.get(key))
                    .ok_or_else(|| format!("模型缺少已解析外观参数 {key}"))?
                    .clone()
            } else {
                match nbt_at(fields.get("$nbt").ok_or("模型绑定缺少来源")?, context)? {
                    Some(value) => value,
                    None => evaluate(
                        fields
                            .get("absent")
                            .ok_or("模型绑定的源 NBT 路径不存在且未声明 absent")?,
                        context,
                        depth + 1,
                    )?,
                }
            };
            if let Some(map) = fields.get("map") {
                let key = match &result {
                    Value::String(v) => v.clone(),
                    Value::Number(v) => v.to_string(),
                    Value::Bool(v) => v.to_string(),
                    _ => return Err("枚举绑定来源必须为标量".into()),
                };
                result = evaluate(
                    map.as_object()
                        .ok_or("模型 map 必须为对象")?
                        .get(&key)
                        .ok_or_else(|| format!("模型绑定未声明枚举值 {key}"))?,
                    context,
                    depth + 1,
                )?;
            }
            if fields.contains_key("scale") || fields.contains_key("offset") {
                let number = |value: &Value| {
                    value
                        .as_f64()
                        .or_else(|| value.as_str()?.parse::<f64>().ok())
                        .filter(|v| v.is_finite())
                        .ok_or("模型数值绑定不是有限数")
                };
                let source = number(&result)?;
                let scale = fields.get("scale").map(number).transpose()?.unwrap_or(1.);
                let offset = fields.get("offset").map(number).transpose()?.unwrap_or(0.);
                result = serde_json::Number::from_f64(source * scale + offset)
                    .map(Value::Number)
                    .ok_or("模型数值绑定结果溢出")?;
            }
            Ok(result)
        }
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| Ok((key.clone(), evaluate(value, context, depth + 1)?)))
            .collect::<Result<Map<_, _>, String>>()
            .map(Value::Object),
        value => Ok(value.clone()),
    }
}
/// 模型层、普通 JSON 元素和插件外观复用同一个参数绑定器。
pub(crate) fn bind_document(document: &Value, context: &Context<'_>) -> Result<Value, String> {
    evaluate(document, context, 0)
}
pub(crate) fn builtin_element_document(name: &str, params: &Value) -> Result<Value, String> {
    static DOCUMENTS: std::sync::OnceLock<Result<BTreeMap<String, Value>, String>> =
        std::sync::OnceLock::new();
    let documents = DOCUMENTS
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../mcedit2-world/assets/block-element-recipes.json"
            ))
            .map_err(|e| format!("原版元素定义无效：{e}"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let document = documents
        .get(name)
        .ok_or_else(|| format!("未声明原版元素定义 {name}"))?;
    bind_document(
        document,
        &Context {
            state: None,
            visual: None,
            params: Some(params.as_object().ok_or("模型参数必须为对象")?),
            catalog: None,
        },
    )
}
impl Template {
    pub(crate) fn compile(
        document: &Value,
        textures: Arc<Map<String, Value>>,
        root: Affine,
        files: &BTreeMap<String, Value>,
    ) -> Result<Self, String> {
        Self::compile_with_catalogs(document, textures, root, &|key| {
            let source = files
                .get(key)
                .ok_or_else(|| format!("缺少模型层目录 {key}"))?;
            Catalog::from_json(&serde_json::to_string(source).map_err(|e| e.to_string())?)
                .map(Arc::new)
        })
    }
    pub(crate) fn compile_with_catalogs(
        document: &Value,
        textures: Arc<Map<String, Value>>,
        root: Affine,
        catalogs: &dyn Fn(&str) -> Result<Arc<Catalog>, String>,
    ) -> Result<Self, String> {
        for key in document.as_object().ok_or("模型层声明必须为对象")?.keys() {
            if !matches!(
                key.as_str(),
                "loader"
                    | "parent"
                    | "textures"
                    | "transform"
                    | "render_type"
                    | "ambientocclusion"
                    | "display"
                    | "gui_light"
                    | "layer"
                    | "catalog"
                    | "poses"
                    | "texture"
                    | "layer_transform"
                    | "visible"
                    | "parts"
                    | "material"
                    | "space"
                    | "block_entity"
                    | "part_transforms"
                    | "entity"
            ) {
                return Err(format!("模型层声明含未知字段 {key}"));
            }
        }
        let catalog = if let Some(catalog) = document.get("catalog") {
            let id = normalize_model(catalog.as_str().ok_or("catalog 必须为模型资源 ID")?)
                .ok_or("模型目录资源 ID 无效")?;
            ModelCatalog::Resource(catalogs(&id)?)
        } else {
            ModelCatalog::Vanilla
        };
        let layer = document.get("layer").ok_or("模型声明缺少 layer")?;
        if let Some(layer) = layer.as_str() {
            let _ = catalog.get()?.parts(layer)?;
        } else if !layer.is_object() {
            return Err("模型 layer 必须为资源 ID 或显式绑定".into());
        }
        if document.get("texture").is_none() {
            return Err("模型层声明缺少 texture".into());
        }
        let state_only = RENDER_FIELDS
            .into_iter()
            .filter_map(|key| document.get(key))
            .all(state_only)
            && textures.values().all(state_only);
        Ok(Self {
            document: Arc::new(document.clone()),
            textures,
            catalog,
            root,
            state_only,
            instances: Arc::new(Mutex::new(Instances::default())),
        })
    }
    pub(crate) fn instantiate(&self, context: &Context<'_>) -> Result<Vec<ObjSelection>, String> {
        self.instance(context).map(|faces| faces.as_ref().clone())
    }
    pub(crate) fn validate(&self, context: &Context<'_>) -> Result<(), String> {
        self.instance(context).map(|_| ())
    }
    fn instance(&self, context: &Context<'_>) -> Result<Arc<Vec<ObjSelection>>, String> {
        let lock = || {
            self.instances
                .lock()
                .map_err(|error| format!("模型层缓存锁异常：{error}"))
        };
        let state = context
            .state
            .filter(|_| self.state_only && context.catalog.is_none());
        if let Some(state) = state {
            let mut cache = lock()?;
            if let Some(index) = cache.entries.iter().position(|(key, _, _)| key == state) {
                let entry = cache.entries.remove(index).ok_or("模型层缓存命中项丢失")?;
                let value = entry.1.clone();
                cache.entries.push_back(entry);
                return value;
            }
        }
        let result = self.instantiate_uncached(context).map(Arc::new);
        if let Some(state) = state {
            let bytes = result.as_ref().map_or_else(
                |error| error.len(),
                |faces| {
                    faces
                        .iter()
                        .map(|face| {
                            std::mem::size_of::<ObjSelection>()
                                + face.texture.len()
                                + face.alpha_mode.as_ref().map_or(0, String::len)
                        })
                        .sum()
                },
            );
            let bytes = bytes
                .checked_add(mcedit2_world::clipboard::state_bytes(state))
                .and_then(|n| {
                    n.checked_add(std::mem::size_of::<(
                        State,
                        Result<Arc<Vec<ObjSelection>>, String>,
                        usize,
                    )>())
                })
                .ok_or("模型缓存计量溢出")?;
            const LIMIT: usize = 4 * 1024 * 1024;
            if bytes <= LIMIT {
                let mut cache = lock()?;
                while cache.bytes + bytes > LIMIT {
                    if let Some((_, _, cost)) = cache.entries.pop_front() {
                        cache.bytes -= cost;
                    } else {
                        break;
                    }
                }
                cache.bytes += bytes;
                cache
                    .entries
                    .push_back((state.clone(), result.clone(), bytes));
            }
        }
        result
    }
    fn instantiate_uncached(&self, context: &Context<'_>) -> Result<Vec<ObjSelection>, String> {
        if let Some(visible) = self.document.get("visible") {
            if !evaluate(visible, context, 0)?
                .as_bool()
                .ok_or("模型 visible 必须为布尔值")?
            {
                return Ok(Vec::new());
            }
        }
        // 身份、版本及字段合同是声明数据，不作为参数表达式再次解释。
        let render = Value::Object(
            RENDER_FIELDS
                .into_iter()
                .filter_map(|key| {
                    self.document
                        .get(key)
                        .map(|value| (key.into(), value.clone()))
                })
                .collect(),
        );
        let doc = evaluate(&render, context, 0)?;
        if let Some(visible) = doc.get("visible") {
            if !visible.as_bool().ok_or("模型 visible 必须为布尔值")? {
                return Ok(Vec::new());
            }
        }
        let layer = doc["layer"].as_str().ok_or("模型 layer 无效")?;
        let poses: BTreeMap<String, PartPose> = serde_json::from_value(
            doc.get("poses")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new())),
        )
        .map_err(|e| format!("模型姿态无效：{e}"))?;
        let catalog = match &self.catalog {
            ModelCatalog::Vanilla => match context.catalog {
                Some(catalog) => catalog,
                None => self.catalog.get()?,
            },
            ModelCatalog::Resource(catalog) => catalog,
        };
        let parts = doc
            .get("parts")
            .map(|parts| {
                let known = catalog.parts(layer)?.collect::<BTreeSet<_>>();
                parts
                    .as_array()
                    .ok_or("模型 parts 必须为数组")?
                    .iter()
                    .map(|part| {
                        let part = part.as_str().ok_or("模型部件路径必须为字符串")?;
                        if !known.contains(part) {
                            return Err(format!("模型层 {layer} 没有部件 {part}"));
                        }
                        Ok(part)
                    })
                    .collect::<Result<BTreeSet<_>, String>>()
            })
            .transpose()?;
        let mut texture = doc["texture"]
            .as_str()
            .ok_or("模型 texture 必须为资源 ID 或别名")?
            .to_owned();
        let mut aliases = BTreeSet::new();
        while let Some(alias) = texture.strip_prefix('#') {
            if !aliases.insert(alias.to_owned()) {
                return Err("模型纹理别名循环".into());
            }
            let value = self
                .textures
                .get(alias)
                .ok_or_else(|| format!("模型缺少纹理别名 #{alias}"))?;
            let value = evaluate(value, context, 0)?;
            texture = value
                .as_str()
                .or_else(|| value.get("sprite").and_then(Value::as_str))
                .ok_or("模型纹理别名无效")?
                .to_owned();
        }
        let texture = normalize_texture(&texture).ok_or("模型纹理资源 ID 无效")?;
        let transform = doc
            .get("layer_transform")
            .map(model_transform)
            .transpose()?
            .unwrap_or(Affine::IDENTITY);
        let part_transforms = doc
            .get("part_transforms")
            .map(|value| {
                let known = catalog.parts(layer)?.collect::<BTreeSet<_>>();
                value
                    .as_object()
                    .ok_or("part_transforms 必须为部件路径映射")?
                    .iter()
                    .map(|(path, value)| {
                        if !known.contains(path.as_str()) {
                            return Err(format!("模型层 {layer} 未声明变换部件 {path}"));
                        }
                        let local = Affine::translation([0.5; 3])
                            .compose(model_transform(value)?)?
                            .compose(Affine::translation([-0.5; 3]))?;
                        Ok((path.as_str(), local))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()
            })
            .transpose()?
            .unwrap_or_default();
        let space = doc
            .get("space")
            .map(|v| v.as_str().ok_or("模型 space 必须为字符串"))
            .transpose()?
            .unwrap_or("block");
        let origin = match space {
            "block" => Affine::IDENTITY,
            "entity" => Affine::translation([0.5; 3]),
            _ => return Err(format!("未知模型坐标空间 {space}")),
        };
        let transform = self
            .root
            .compose(origin)?
            .compose(transform)?
            .compose(Affine::translation([-0.5; 3]))?;
        let material = doc.get("material");
        if let Some(material) = material {
            if material
                .as_object()
                .ok_or("模型 material 必须为对象")?
                .keys()
                .any(|k| {
                    !matches!(
                        k.as_str(),
                        "alpha"
                            | "alpha_test"
                            | "depth_write"
                            | "cutoff"
                            | "double_sided"
                            | "mipmap"
                            | "tint"
                            | "view_offset_z"
                            | "ordered"
                            | "texture_source"
                    )
                })
            {
                return Err("模型 material 含未知字段".into());
            }
        }
        let flag = |key, default| {
            material
                .and_then(|v| v.get(key))
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| format!("模型材质 {key} 必须为布尔值"))
                })
                .unwrap_or(Ok(default))
        };
        let alpha = material
            .and_then(|m| m.get("alpha"))
            .map(|v| v.as_str().ok_or("模型 alpha 必须为字符串"))
            .transpose()?
            .unwrap_or("MASK");
        if !matches!(alpha, "OPAQUE" | "MASK" | "BLEND") {
            return Err(format!("模型 alpha 无效：{alpha}"));
        }
        let cutoff = material
            .and_then(|m| m.get("cutoff"))
            .map(|v| {
                v.as_f64()
                    .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
                    .map(|v| v as f32)
                    .ok_or("模型 cutoff 必须为 0..1 有限数")
            })
            .transpose()?
            .unwrap_or(0.1);
        let tint: [f32; 4] = material
            .and_then(|m| m.get("tint"))
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| format!("模型 tint 无效：{e}")))
            .transpose()?
            .unwrap_or([1.; 4]);
        if !tint
            .into_iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(&v))
        {
            return Err("模型 tint 必须为四项 0..1 有限数".into());
        }
        let double_sided = flag("double_sided", false)?;
        let texture_source = material
            .and_then(|m| m.get("texture_source"))
            .map(|value| value.as_str().ok_or("模型 texture_source 必须为字符串"))
            .transpose()?
            .unwrap_or("simple");
        let texture_flags = match texture_source {
            "simple" => crate::material_mode::SIMPLE_TEXTURE,
            "atlas" => 0,
            _ => return Err(format!("未知模型纹理来源 {texture_source}")),
        };
        let material_flags = if flag("alpha_test", false)? {
            crate::material_mode::encode(0, Some("MASK"), cutoff, true) & !3
        } else {
            0
        } | if flag("depth_write", false)? {
            crate::material_mode::BLEND_DEPTH_WRITE
        } else {
            0
        } | if flag("mipmap", false)? {
            0
        } else {
            crate::material_mode::NO_MIPMAP
        } | if flag("view_offset_z", false)? {
            crate::material_mode::VIEW_OFFSET_Z
        } else {
            0
        } | if flag("ordered", false)? {
            crate::material_mode::ORDERED_OPAQUE
        } else {
            0
        } | texture_flags;
        let quads = catalog.bake(layer, &poses)?;
        let mut result = Vec::new();
        for quad in quads {
            if parts.as_ref().is_some_and(|parts| {
                !parts.iter().any(|part| {
                    quad.part == *part
                        || quad
                            .part
                            .strip_prefix(*part)
                            .is_some_and(|suffix| suffix.starts_with('/'))
                })
            }) {
                continue;
            }
            let local = part_transforms
                .iter()
                .filter(|(path, _)| {
                    path.is_empty()
                        || quad.part == **path
                        || quad
                            .part
                            .strip_prefix(**path)
                            .is_some_and(|tail| tail.starts_with('/'))
                })
                .max_by_key(|(path, _)| path.len())
                .map_or(Affine::IDENTITY, |(_, transform)| *transform);
            let combined = transform.compose(local)?;
            let points = quad.corners.map(|p| combined.point([p[0], p[1], p[2]]));
            if !points
                .iter()
                .flatten()
                .all(|v| v.is_finite() && v.abs() <= 1.0e7)
            {
                return Err("模型层变换后坐标无效或超出范围".into());
            }
            let order = if combined.determinant() < 0. {
                [0, 3, 2, 1]
            } else {
                [0, 1, 2, 3]
            };
            for indices in crate::world_mesh::TRIANGLES {
                let indices = indices.map(|i| order[i]);
                result.push(ObjSelection {
                    xyz: indices.map(|i| points[i]),
                    uv: indices.map(|i| [quad.corners[i][3], quad.corners[i][4]]),
                    texture: texture.clone(),
                    tint,
                    colors: [[1.; 4]; 3],
                    alpha_mode: Some(alpha.into()),
                    alpha_cutoff: cutoff,
                    double_sided,
                    material_flags,
                    cull: None,
                    render_type: None,
                    tint_index: None,
                });
            }
        }
        Ok(result)
    }
}

/// 图形后端只接收通用后端烘焙出的源面；原版外观适配不再复制材质和 UV 逻辑。
pub(crate) fn world_quads(
    triangles: &[ObjSelection],
    atlas: &crate::textures::TextureAtlas,
    textures: bool,
) -> Result<Vec<crate::world_mesh::WorldQuad>, String> {
    if !triangles.len().is_multiple_of(2) {
        return Err("模型层三角形不组成完整四角面".into());
    }
    let mut quads = Vec::new();
    for pair in triangles.chunks_exact(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.xyz[0] != b.xyz[0]
            || a.xyz[2] != b.xyz[1]
            || a.uv[0] != b.uv[0]
            || a.uv[2] != b.uv[1]
            || a.texture != b.texture
        {
            return Err("模型层四角拓扑不一致".into());
        }
        let slot = if textures {
            atlas
                .material(&a.texture, a.material_flags)
                .ok_or_else(|| format!("模型层图集缺少 {}", a.texture))?
        } else {
            &atlas.white
        };
        let quad = crate::world_mesh::WorldQuad {
            xyz: [a.xyz[0], a.xyz[1], a.xyz[2], b.xyz[2]],
            uv: if textures {
                [a.uv[0], a.uv[1], a.uv[2], b.uv[2]]
            } else {
                [[0.5; 2]; 4]
            },
            rgba: [a.tint; 4],
            mode: crate::material_mode::encode(
                1,
                a.alpha_mode.as_deref(),
                a.alpha_cutoff,
                a.double_sided,
            ) | a.material_flags,
            atlas_page: slot.page,
            light: crate::lighting::FULLBRIGHT,
        };
        let sampler = if textures {
            atlas.native_sampler(slot, quad.mode)?
        } else {
            None
        };
        crate::static_triangle_uv::bind_quad(
            quad,
            slot,
            sampler,
            Some(atlas.resource_budget()),
            |face| quads.push(face),
        )?;
    }
    Ok(quads)
}
pub(crate) fn builtin_quads(
    name: &str,
    params: &Map<String, Value>,
    atlas: &crate::textures::TextureAtlas,
    textures: bool,
) -> Result<Vec<crate::world_mesh::WorldQuad>, String> {
    builtin_quads_with_catalog(name, params, atlas, textures, None)
}
pub(crate) fn builtin_quads_with_catalog(
    name: &str,
    params: &Map<String, Value>,
    atlas: &crate::textures::TextureAtlas,
    textures: bool,
    catalog: Option<&Catalog>,
) -> Result<Vec<crate::world_mesh::WorldQuad>, String> {
    static TEMPLATES: std::sync::OnceLock<Result<BTreeMap<String, Template>, String>> =
        std::sync::OnceLock::new();
    let templates = TEMPLATES
        .get_or_init(|| {
            let mut documents = BTreeMap::new();
            for source in [
                include_str!("../../mcedit2-world/assets/block-visual-recipes-1.21.1.json"),
                include_str!("../../mcedit2-world/assets/entity-visual-recipes-1.21.1.json"),
            ] {
                let entries: BTreeMap<String, Value> =
                    serde_json::from_str(source).map_err(|e| format!("原版外观定义无效：{e}"))?;
                for (name, doc) in entries {
                    if documents.insert(name.clone(), doc).is_some() {
                        return Err(format!("原版外观定义重复：{name}"));
                    }
                }
            }
            documents
                .into_iter()
                .map(|(name, doc)| {
                    Ok((
                        name,
                        Template::compile(
                            &doc,
                            Arc::new(Map::new()),
                            Affine::IDENTITY,
                            &BTreeMap::new(),
                        )?,
                    ))
                })
                .collect()
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let template = templates
        .get(name)
        .ok_or_else(|| format!("未声明原版外观定义 {name}"))?;
    let faces = template.instantiate(&Context {
        state: None,
        visual: None,
        params: Some(params),
        catalog,
    })?;
    world_quads(&faces, atlas, textures)
}
