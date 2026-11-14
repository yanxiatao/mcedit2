//! 声明式模型图：父模型、组合子模型、可见性、独立纹理空间与根变换。
use super::{normalize_model, vanilla_parent_elements};
use crate::root_transform::Affine;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug)]
pub(super) struct Element {
    pub document: Value,
    pub textures: Arc<Map<String, Value>>,
    pub root: Affine,
    pub render_type: Option<crate::textures::AlphaMode>,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Model {
    pub elements: Vec<Element>,
    pub triangles: Vec<super::ObjSelection>,
    pub layers: Vec<crate::layer_renderer::Template>,
    /// 同一模型首次用于 UV 锁定时验证逆矩阵，并保留失败，避免逐体素重复诊断。
    pub uv_lock: OnceLock<Result<(), String>>,
}
pub(super) struct MeshSpec<'a> {
    pub document: &'a Value,
    pub textures: Arc<Map<String, Value>>,
    pub visibility: BTreeMap<String, bool>,
    pub root: Affine,
    pub render_type: Option<crate::textures::AlphaMode>,
}
pub(super) fn supported(loader: &str) -> bool {
    matches!(
        loader,
        "neoforge:composite"
            | "forge:composite"
            | "neoforge:elements"
            | "forge:elements"
            | "neoforge:empty"
            | "forge:empty"
            | "neoforge:separate_transforms"
            | "forge:separate_transforms"
            | "neoforge:obj"
            | "forge:obj"
            | "mcedit2:entity_layer"
    )
}
struct Inputs<'a> {
    context: Option<&'a str>,
    files: &'a BTreeMap<String, Value>,
    resolve: &'a dyn Fn(MeshSpec<'_>) -> Result<Vec<super::ObjSelection>, String>,
    catalogs: &'a dyn Fn(&str) -> Result<Arc<mcedit2_world::entity_model::Catalog>, String>,
}
pub(super) fn compile(
    files: &BTreeMap<String, Value>,
    key: &str,
    resolve: &impl Fn(MeshSpec<'_>) -> Result<Vec<super::ObjSelection>, String>,
    catalogs: &impl Fn(&str) -> Result<Arc<mcedit2_world::entity_model::Catalog>, String>,
) -> Result<Model, String> {
    compile_context(files, key, resolve, catalogs, None)
}
pub(super) fn compile_item(
    files: &BTreeMap<String, Value>,
    key: &str,
    context: &str,
    resolve: &impl Fn(MeshSpec<'_>) -> Result<Vec<super::ObjSelection>, String>,
    catalogs: &impl Fn(&str) -> Result<Arc<mcedit2_world::entity_model::Catalog>, String>,
) -> Result<Model, String> {
    compile_context(files, key, resolve, catalogs, Some(context))
}
fn compile_context(
    files: &BTreeMap<String, Value>,
    key: &str,
    resolve: &impl Fn(MeshSpec<'_>) -> Result<Vec<super::ObjSelection>, String>,
    catalogs: &impl Fn(&str) -> Result<Arc<mcedit2_world::entity_model::Catalog>, String>,
    context: Option<&str>,
) -> Result<Model, String> {
    let doc = files
        .get(key)
        .ok_or_else(|| format!("模型资源不存在：{key}"))?;
    let mut model = Model::default();
    expand(
        &Inputs {
            context,
            files,
            resolve,
            catalogs,
        },
        doc,
        key,
        &BTreeSet::new(),
        Affine::IDENTITY,
        &mut model,
    )?;
    Ok(model)
}

pub(super) struct ParentChain<'a> {
    pub chain: Vec<&'a Value>,
    pub names: Vec<Option<String>>,
    pub fallback: Option<&'static Vec<Value>>,
}
pub(super) fn parent_chain<'a>(
    files: &'a BTreeMap<String, Value>,
    doc: &'a Value,
    label: &str,
    ancestors: &BTreeSet<String>,
) -> Result<ParentChain<'a>, String> {
    let mut visited = BTreeSet::new();
    let mut chain = vec![doc];
    let inline = label.contains("#/children/") || label.contains("#/base");
    let mut names = vec![(!inline).then(|| label.to_owned())];
    let mut fallback = None;
    if !inline && (ancestors.contains(label) || !visited.insert(label.to_owned())) {
        return Err(format!("模型图存在循环：{label}"));
    }
    while let Some(parent) = chain.last().and_then(|d| d.get("parent")) {
        let key = normalize_model(parent.as_str().ok_or("parent 必须是资源 ID")?)
            .ok_or("parent 资源 ID 无效")?;
        if ancestors.contains(&key) || !visited.insert(key.clone()) {
            return Err(format!("父模型存在循环：{label} -> {key}"));
        }
        if let Some(doc) = files.get(&key) {
            chain.push(doc);
            names.push(Some(key));
        } else if let Some(elements) = vanilla_parent_elements(&key) {
            fallback = Some(elements);
            break;
        } else {
            return Err(format!("父模型资源不存在：{label} -> {key}"));
        }
    }
    Ok(ParentChain {
        chain,
        names,
        fallback,
    })
}
pub(super) fn merged_textures(chain: &[&Value]) -> Result<Map<String, Value>, String> {
    let mut textures = Map::new();
    for doc in chain.iter().rev() {
        if let Some(map) = doc.get("textures") {
            textures.extend(map.as_object().ok_or("textures 必须为对象")?.clone());
        }
    }
    Ok(textures)
}
fn expand(
    inputs: &Inputs<'_>,
    doc: &Value,
    label: &str,
    ancestors: &BTreeSet<String>,
    outer: Affine,
    out: &mut Model,
) -> Result<(), String> {
    let files = inputs.files;
    let resolve = inputs.resolve;
    let ParentChain {
        chain,
        names,
        fallback,
    } = parent_chain(files, doc, label, ancestors)?;
    let root = chain
        .iter()
        .find_map(|doc| doc.get("transform"))
        .map(Affine::parse)
        .transpose()?
        .unwrap_or(Affine::IDENTITY);
    let root = outer.compose(root)?;
    let render_type = chain
        .iter()
        .find_map(|doc| doc.get("render_type"))
        .map(|value| {
            crate::textures::AlphaMode::from_render_type(
                value.as_str().ok_or("render_type 必须为资源 ID")?,
            )
        })
        .transpose()?;
    let textures = Arc::new(merged_textures(&chain)?);
    let custom = chain
        .iter()
        .find(|doc| doc.get("loader").is_some() || doc.get("mcedit2:static_renderer").is_some());
    let mut active = ancestors.clone();
    if let Some(position) = chain
        .iter()
        .position(|doc| doc.get("loader").is_some() || doc.get("mcedit2:static_renderer").is_some())
    {
        active.extend(names.iter().take(position + 1).flatten().cloned());
    }
    let loader = custom.and_then(|doc| doc.get("loader"));
    if custom.is_some() && loader.is_none() {
        return Err(format!("模型需要静态扩展后端：{label}"));
    }
    let loader = loader
        .map(|v| v.as_str().ok_or("loader 必须为资源 ID"))
        .transpose()?;
    match loader {
        Some("mcedit2:entity_layer") => {
            out.layers
                .push(crate::layer_renderer::Template::compile_with_catalogs(
                    custom.unwrap(),
                    textures,
                    root,
                    inputs.catalogs,
                )?);
            Ok(())
        }
        Some("neoforge:empty" | "forge:empty") => Ok(()),
        Some("neoforge:composite" | "forge:composite") => {
            let declaration = custom.unwrap();
            let children = declaration
                .get("children")
                .and_then(Value::as_object)
                .filter(|c| !c.is_empty())
                .ok_or("组合模型必须声明非空 children")?;
            if let Some(order) = declaration.get("item_render_order") {
                for name in order.as_array().ok_or("item_render_order 必须为数组")? {
                    if !name.as_str().is_some_and(|s| children.contains_key(s)) {
                        return Err("item_render_order 引用了未声明的子模型".into());
                    }
                }
            }
            for (name, child) in children {
                let mut visible = true;
                for doc in &chain {
                    if let Some(visibility) = doc.get("visibility") {
                        let visibility = visibility.as_object().ok_or("visibility 必须为对象")?;
                        if let Some(value) = visibility.get(name) {
                            visible = value.as_bool().ok_or("子模型可见性必须是布尔值")?;
                            break;
                        }
                    }
                }
                if visible {
                    if !child.is_object() {
                        return Err(format!("组合子模型 {name} 必须是对象"));
                    }
                    expand(
                        inputs,
                        child,
                        &format!("{label}#/children/{name}"),
                        &active,
                        root,
                        out,
                    )?;
                }
            }
            Ok(())
        }
        Some("neoforge:separate_transforms" | "forge:separate_transforms") => {
            let base = custom
                .unwrap()
                .get("base")
                .filter(|v| v.is_object())
                .ok_or("独立变换模型必须声明 base 对象")?;
            let base = if let Some(context) = inputs.context {
                custom
                    .unwrap()
                    .get("perspectives")
                    .and_then(|p| p.get(context))
                    .unwrap_or(base)
            } else {
                base
            };
            expand(inputs, base, &format!("{label}#/base"), &active, root, out)
        }
        Some("neoforge:obj" | "forge:obj") => {
            let mut visibility = BTreeMap::new();
            for doc in chain.iter().rev() {
                if let Some(map) = doc.get("visibility") {
                    for (name, value) in map.as_object().ok_or("OBJ visibility 必须为对象")? {
                        visibility.insert(
                            name.clone(),
                            value.as_bool().ok_or("OBJ 组件可见性必须为布尔值")?,
                        );
                    }
                }
            }
            out.triangles.extend(resolve(MeshSpec {
                document: custom.unwrap(),
                textures,
                visibility,
                root,
                render_type,
            })?);
            Ok(())
        }
        Some(loader) if !matches!(loader, "neoforge:elements" | "forge:elements") => {
            Err(format!("模型渲染器尚未支持：{loader} ({label})"))
        }
        _ => {
            let elements = if loader.is_some() {
                custom
                    .unwrap()
                    .get("elements")
                    .and_then(Value::as_array)
                    .ok_or("elements 渲染器必须声明 elements 数组")?
                    .as_slice()
            } else {
                let mut chosen = None;
                for doc in &chain {
                    if let Some(e) = doc.get("elements") {
                        let e = e.as_array().ok_or("elements 必须为数组")?;
                        if !e.is_empty() {
                            chosen = Some(e);
                            break;
                        }
                    }
                }
                chosen.or(fallback).map(Vec::as_slice).unwrap_or(&[])
            };
            for element in elements {
                if !element.is_object() {
                    return Err("模型元素必须为对象".into());
                }
                out.elements.push(Element {
                    document: element.clone(),
                    textures: Arc::clone(&textures),
                    root,
                    render_type,
                });
            }
            Ok(())
        }
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn compile(files: &BTreeMap<String, Value>, key: &str) -> Result<Model, String> {
        super::compile(
            files,
            key,
            &|_| Err("该测试未载入外部网格资源".into()),
            &|_| Err("该分类未载入外部模型目录".into()),
        )
    }
    fn files(entries: &[(&str, &str)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(id, doc)| {
                (
                    format!("model:test:{id}"),
                    serde_json::from_str(doc).unwrap(),
                )
            })
            .collect()
    }

    fn composites_inherit_visibility_but_children_keep_independent_textures() {
        let mut source = files(&[
            (
                "cube",
                r##"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}"##,
            ),
            (
                "parent",
                r##"{"loader":"neoforge:composite","children":{"left":{"parent":"test:block/cube","textures":{"all":"test:block/left"}},"right":{"parent":"test:block/cube","textures":{"all":"test:block/right"}}},"visibility":{"right":false},"textures":{"all":"test:block/outer"}}"##,
            ),
            (
                "child",
                r#"{"parent":"test:block/parent","visibility":{"left":false,"right":true},"transform":{"translation":[1,0,0]}}"#,
            ),
        ]);
        let parent = compile(&source, "model:test:parent").unwrap();
        assert_eq!(parent.elements.len(), 1);
        assert_eq!(parent.elements[0].textures["all"], "test:block/left");
        let child = compile(&source, "model:test:child").unwrap();
        assert_eq!(child.elements.len(), 1);
        assert_eq!(child.elements[0].textures["all"], "test:block/right");
        assert_eq!(child.elements[0].root.point([0.; 3]), [1., 0., 0.]);
        for i in 0..96 {
            let parent = if i == 95 {
                "child".to_owned()
            } else {
                format!("chain{}", i + 1)
            };
            source.insert(
                format!("model:test:chain{i}"),
                serde_json::json!({"parent":format!("test:block/{parent}")}),
            );
        }
        let inherited = compile(&source, "model:test:chain0").unwrap();
        assert_eq!(inherited.elements.len(), child.elements.len());
        assert_eq!(inherited.elements[0].textures, child.elements[0].textures);
        assert_eq!(inherited.elements[0].root.point([0.; 3]), [1., 0., 0.]);
    }

    fn base_empty_and_nested_models_do_not_invent_geometry() {
        let source = files(&[
            ("empty", r#"{"loader":"neoforge:empty"}"#),
            (
                "base",
                r#"{"loader":"neoforge:separate_transforms","base":{"loader":"neoforge:elements","elements":[]},"perspectives":{"gui":{"parent":"test:block/missing"}}}"#,
            ),
            (
                "cycle",
                r#"{"loader":"neoforge:composite","children":{"recursive":{"parent":"test:block/cycle"}}}"#,
            ),
            (
                "bad",
                r#"{"loader":"neoforge:composite","children":{"part":{"parent":"test:block/missing"}}}"#,
            ),
        ]);
        assert!(compile(&source, "model:test:empty")
            .unwrap()
            .elements
            .is_empty());
        assert!(compile(&source, "model:test:base")
            .unwrap()
            .elements
            .is_empty());
        assert!(compile(&source, "model:test:cycle")
            .unwrap_err()
            .contains("循环"));
        assert!(compile(&source, "model:test:bad")
            .unwrap_err()
            .contains("不存在"));
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Models {
            composites_inherit_visibility_but_children_keep_independent_textures();
            base_empty_and_nested_models_do_not_invent_geometry();
        }
    }
}
