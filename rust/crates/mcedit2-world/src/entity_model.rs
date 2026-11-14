//! 原版模型层的无图形 API 编译器：部件层级、四角和 UV 均来自公开客户端接口。
//! 输出仍为模型局部坐标，不推测实体类型、皮肤或世界存储版本。
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
#[path = "entity_model_legacy_horse.rs"]
mod legacy_horse;

/// 原版普通站立生物的模型局部坐标到实体脚部相对坐标。
/// 顺序来自 1.21.1 LivingEntityRenderer：Y(180-yaw)、scale(-1,-1,1)、translate(0,-1.501,0)。
/// 睡眠、死亡翻转、骑乘、幼年缩放和渲染器附加层不属于这个变换。
pub fn living_offset(point: [f32; 3], yaw_degrees: f32) -> Result<[f32; 3], String> {
    if !point.into_iter().chain([yaw_degrees]).all(f32::is_finite) {
        return Err("生物模型变换含非有限坐标或偏航".into());
    }
    let (s, c) = (180.0 - yaw_degrees).to_radians().sin_cos();
    let [x, y, z] = [-point[0], 1.501 - point[1], point[2]];
    let result = [c * x + s * z, y, -s * x + c * z];
    if !result.into_iter().all(f32::is_finite) {
        return Err("生物模型变换结果溢出".into());
    }
    Ok(result)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    pivot: [f32; 3],
    rotation: [f32; 3],
    scale: [f32; 3],
    visible: bool,
    skip_draw: bool,
    cubes: BTreeMap<usize, Vec<[f32; 5]>>,
}
/// 覆盖源部件的静态姿态；旋转是弧度，pivot 是客户端的像素单位（16 像素 / 格）。
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartPose {
    pub pivot: Option<[f32; 3]>,
    pub rotation: Option<[f32; 3]>,
    pub scale: Option<[f32; 3]>,
    pub visible: Option<bool>,
    pub skip_draw: Option<bool>,
}

#[test]
fn test_tests_all_client_layers_and_pose_overrides_match_actual_vertices() {
    tests::all_client_layers_and_pose_overrides_match_actual_vertices();
}

#[cfg(test)]
mod tests {
    use super::*;
    type Faces = BTreeMap<String, BTreeMap<usize, Vec<[f32; 5]>>>;
    #[derive(Deserialize)]
    struct Reference {
        version: String,
        reference: BTreeMap<String, Faces>,
        poses: Vec<Case>,
        living: Vec<Living>,
    }
    #[derive(Deserialize)]
    struct Living {
        layer: String,
        yaw: f32,
        reference: Faces,
    }
    #[derive(Deserialize)]
    struct Case {
        layer: String,
        overrides: BTreeMap<String, PartPose>,
        reference: Faces,
    }
    fn compare(
        catalog: &Catalog,
        layer: &str,
        overrides: &BTreeMap<String, PartPose>,
        expected: &Faces,
    ) -> (usize, f32) {
        let quads = catalog.bake(layer, overrides).unwrap();
        let mut actual: Faces = BTreeMap::new();
        for quad in &quads {
            actual
                .entry(quad.part.clone())
                .or_default()
                .entry(quad.cube)
                .or_default()
                .extend(quad.corners);
        }
        assert_eq!(actual.len(), expected.len(), "模型层 {layer} 的源部件数");
        let mut maximum = 0_f32;
        for (part, cubes) in expected {
            let actual = &actual[part];
            assert_eq!(actual.len(), cubes.len());
            for (cube, vertices) in cubes {
                let actual = &actual[cube];
                assert_eq!(actual.len(), vertices.len(), "{layer} {part}/{cube}");
                for (actual, expected) in actual.iter().zip(vertices) {
                    assert_eq!(actual[3..], expected[3..], "{layer} {part}/{cube} 的 UV");
                    for i in 0..3 {
                        maximum = maximum.max((actual[i] - expected[i]).abs());
                    }
                }
            }
        }
        assert!(maximum <= 0.00002, "{layer} 逐角点偏差 {maximum}");
        (quads.len(), maximum)
    }
    pub(super) fn all_client_layers_and_pose_overrides_match_actual_vertices() {
        let catalog = Catalog::java_1_21_1().unwrap();
        let reference: Reference = serde_json::from_str(include_str!(
            "../tests/fixtures/entity-layer-reference-1.21.1.json"
        ))
        .unwrap();
        assert_eq!(catalog.version, reference.version);
        assert_eq!(
            catalog.layers().collect::<Vec<_>>(),
            reference
                .reference
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        let mut count = 0;
        let mut maximum = 0_f32;
        for (layer, expected) in &reference.reference {
            let (quads, delta) = compare(catalog, layer, &BTreeMap::new(), expected);
            count += quads;
            maximum = maximum.max(delta);
        }
        for case in &reference.poses {
            compare(catalog, &case.layer, &case.overrides, &case.reference);
        }
        let mut living_maximum = 0_f32;
        for case in &reference.living {
            let quads = catalog.bake(&case.layer, &BTreeMap::new()).unwrap();
            let mut actual: Faces = BTreeMap::new();
            for quad in quads {
                let vertices = actual
                    .entry(quad.part)
                    .or_default()
                    .entry(quad.cube)
                    .or_default();
                for p in quad.corners {
                    let [x, y, z] = living_offset([p[0], p[1], p[2]], case.yaw).unwrap();
                    vertices.push([x, y, z, p[3], p[4]]);
                }
            }
            assert_eq!(actual.len(), case.reference.len());
            for (part, cubes) in &case.reference {
                assert_eq!(actual[part].len(), cubes.len());
                for (cube, expected) in cubes {
                    assert_eq!(actual[part][cube].len(), expected.len());
                    for (a, e) in actual[part][cube].iter().zip(expected) {
                        for i in 0..5 {
                            living_maximum = living_maximum.max((a[i] - e[i]).abs());
                            assert!(
                                (a[i] - e[i]).abs() < 0.00002,
                                "生物全局变换 yaw={}: {a:?} != {e:?}",
                                case.yaw
                            );
                        }
                    }
                }
            }
        }
        assert!(living_offset([0.; 3], f32::INFINITY).is_err());
        let mut hidden = BTreeMap::from([(
            String::new(),
            PartPose {
                visible: Some(false),
                ..Default::default()
            },
        )]);
        assert!(catalog
            .bake("minecraft:frog#main", &hidden)
            .unwrap()
            .is_empty());
        hidden.insert("/nonexistent".into(), PartPose::default());
        assert!(catalog.bake("minecraft:frog#main", &hidden).is_err());
        assert!(catalog
            .bake("sample:no_model#main", &BTreeMap::new())
            .is_err());
        eprintln!(
            "实际客户端模型参照：{} 层 / {count} 四角面 / {} 组合姿态 / {} 生物全局变换，最大坐标偏差 {maximum} / {living_maximum}",
            catalog.layers.len(),
            reference.poses.len(),
            reference.living.len()
        );
    }
}
/// 一个真实四角面，保留模型层部件路径与源 Cuboid 编号，不伪造方块归属。
#[derive(Clone, Debug, PartialEq)]
pub struct ModelQuad {
    pub part: String,
    pub cube: usize,
    /// xyz 是模型坐标（格），uv 是原始整张实体贴图中的归一化坐标。
    pub corners: [[f32; 5]; 4],
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: String,
    #[serde(deserialize_with = "read_layers")]
    layers: BTreeMap<String, Arc<BTreeMap<String, Part>>>,
    #[serde(default)]
    pub legacy: Option<Legacy>,
}
/// 准确旧客户端公开 API 的幼年偏移和羊毛颜色；不套用现代规则。
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Legacy {
    pub young_cow: [f32; 2],
    pub young_pig: [f32; 2],
    pub sheep_colors: [[f32; 3]; 16],
}
fn read_layers<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Arc<BTreeMap<String, Part>>>, D::Error> {
    let layers = BTreeMap::<String, BTreeMap<String, Part>>::deserialize(deserializer)?;
    Ok(layers
        .into_iter()
        .map(|(key, parts)| (key, Arc::new(parts)))
        .collect())
}
impl Catalog {
    /// 模型绑定以实体载荷的实际版本选择；未知版本保留明确诊断。
    pub fn for_data_version(version: Option<i32>) -> Result<&'static Self, String> {
        static OLD: OnceLock<Result<Catalog, String>> = OnceLock::new();
        static V120: OnceLock<Result<Catalog, String>> = OnceLock::new();
        match version {
            Some(3955) => Self::java_1_21_1().map_err(str::to_owned),
            Some(1343) => OLD
                .get_or_init(|| {
                    let mut catalog =
                        Self::from_json(include_str!("../assets/entity-layers-1.12.2.json"))?;
                    // bpp 与 fvh 的实际构造均为 UV(22,0)、10×20×2、膨胀 1、右翅镜像、64×32。
                    // 只共享静态 Cuboid；旧版幼年外层和保存姿态仍由对应渲染适配解释。
                    let modern = Self::java_1_21_1()?;
                    let wing = modern
                        .layers
                        .get("minecraft:elytra#main")
                        .ok_or("共享鞘翅模型缺失")?;
                    catalog
                        .layers
                        .insert("minecraft:elytra#main".into(), Arc::clone(wing));
                    // brk 的全部狼 Cuboid / UV 与现代定义一致；旧 head / tail 的直接几何拆成恒等子部件。
                    // 保存姿态没有原生未持久化的头部 / 尾部摇晃，相应静态局部旋转仍由动物适配解释。
                    let wolf = modern
                        .layers
                        .get("minecraft:wolf#main")
                        .ok_or("共享狼模型缺失")?;
                    catalog
                        .layers
                        .insert("minecraft:wolf#main".into(), Arc::clone(wolf));
                    // bqc(0 / .5) 的完整构造与现代羊驼层一致，贴图路径仍由旧格式适配。
                    for key in ["minecraft:llama#main", "minecraft:llama#decor"] {
                        let layer = modern.layers.get(key).ok_or("共享羊驼模型缺失")?;
                        catalog.layers.insert(key.into(), Arc::clone(layer));
                    }
                    let horse = legacy_horse::model();
                    for name in ["horse", "donkey", "mule", "zombie_horse", "skeleton_horse"] {
                        catalog
                            .layers
                            .insert(format!("minecraft:{name}#main"), Arc::clone(&horse));
                    }
                    Ok(catalog)
                })
                .as_ref()
                .map_err(Clone::clone),
            Some(3465) => V120
                .get_or_init(|| {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Shared {
                        version: String,
                        shared_layers: Vec<String>,
                        layers: BTreeMap<String, BTreeMap<String, Part>>,
                    }
                    let source: Shared =
                        serde_json::from_str(include_str!("../assets/entity-layers-1.20.1.json"))
                            .map_err(|e| e.to_string())?;
                    let base = Self::java_1_21_1()?;
                    let mut layers = source
                        .layers
                        .into_iter()
                        .map(|(key, parts)| (key, Arc::new(parts)))
                        .collect::<BTreeMap<_, _>>();
                    for key in source.shared_layers {
                        let value = base
                            .layers
                            .get(&key)
                            .ok_or_else(|| format!("已核实的共享模型层缺失：{key}"))?
                            .clone();
                        if layers.insert(key, value).is_some() {
                            return Err("共享模型层重复".into());
                        }
                    }
                    Catalog {
                        version: source.version,
                        layers,
                        legacy: None,
                    }
                    .checked()
                })
                .as_ref()
                .map_err(Clone::clone),
            _ => Err(format!("DataVersion {version:?} 尚无已核对的实体模型绑定")),
        }
    }
    /// 同一份不可变目录可供原生渲染、资源校验和插件宿主使用。
    pub fn java_1_21_1() -> Result<&'static Self, &'static str> {
        static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();
        CATALOG
            .get_or_init(|| Self::from_json(include_str!("../assets/entity-layers-1.21.1.json")))
            .as_ref()
            .map_err(String::as_str)
    }
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > 64 * 1024 * 1024 {
            return Err("实体模型目录超过 64 MiB".into());
        }
        let catalog: Self =
            serde_json::from_str(json).map_err(|e| format!("实体模型目录无效：{e}"))?;
        catalog.checked()
    }
    fn checked(self) -> Result<Self, String> {
        let catalog = self;
        if catalog.version.is_empty() || catalog.layers.is_empty() {
            return Err("实体模型目录缺少版本或模型层".into());
        }
        for (name, parts) in &catalog.layers {
            if !parts.contains_key("") {
                return Err(format!("模型层 {name} 缺少根部件"));
            }
            for (path, part) in parts.iter() {
                if !path.is_empty()
                    && (!path.starts_with('/') || path.ends_with('/') || path.contains("//"))
                    || path.split('/').count() > 64
                {
                    return Err(format!("模型层 {name} 部件路径无效：{path}"));
                }
                if let Some((parent, _)) = path.rsplit_once('/') {
                    if !parts.contains_key(parent) {
                        return Err(format!("模型层 {name} 部件 {path} 的父部件不存在"));
                    }
                }
                if !part
                    .pivot
                    .iter()
                    .chain(&part.rotation)
                    .chain(&part.scale)
                    .all(|n| n.is_finite())
                {
                    return Err(format!("模型层 {name} 部件 {path} 的变换含非有限数"));
                }
                for (cube, vertices) in &part.cubes {
                    if !vertices.len().is_multiple_of(4)
                        || !vertices.iter().flatten().all(|n| n.is_finite())
                    {
                        return Err(format!("模型层 {name} 的 {path}/{cube} 面数据无效"));
                    }
                }
            }
        }
        Ok(catalog)
    }
    pub fn layers(&self) -> impl Iterator<Item = &str> {
        self.layers.keys().map(String::as_str)
    }
    pub fn parts(&self, layer: &str) -> Result<impl Iterator<Item = &str>, String> {
        Ok(self
            .layers
            .get(layer)
            .ok_or_else(|| format!("实体模型层不存在：{layer}"))?
            .keys()
            .map(String::as_str))
    }
    /// 将真实部件局部坐标（格）沿其完整父链转换到模型坐标。
    /// 挂接手持物与装备共用烘焙的 pivot / scale / rotation；部件不可见不删除其坐标系。
    pub fn attachment_point(
        &self,
        layer: &str,
        path: &str,
        overrides: &BTreeMap<String, PartPose>,
        point: [f32; 3],
    ) -> Result<[f32; 3], String> {
        self.attachment_point_chain(layer, path, overrides, point, true)
    }
    /// 旧 ModelRenderer.postRender 只提交当前部件，不隐式继承父链或 render 的 offset。
    pub fn local_attachment_point(
        &self,
        layer: &str,
        path: &str,
        overrides: &BTreeMap<String, PartPose>,
        point: [f32; 3],
    ) -> Result<[f32; 3], String> {
        self.attachment_point_chain(layer, path, overrides, point, false)
    }
    fn attachment_point_chain(
        &self,
        layer: &str,
        path: &str,
        overrides: &BTreeMap<String, PartPose>,
        point: [f32; 3],
        parents: bool,
    ) -> Result<[f32; 3], String> {
        let parts = self
            .layers
            .get(layer)
            .ok_or_else(|| format!("实体模型层不存在：{layer}"))?;
        if !parts.contains_key(path) {
            return Err(format!("模型层 {layer} 未声明挂接部件 {path}"));
        }
        if !point.into_iter().all(f32::is_finite) {
            return Err("挂接点含非有限坐标".into());
        }
        validate_poses(layer, parts, overrides)?;
        let mut result = point;
        let mut current = path;
        loop {
            result = part_point(result, &parts[current], overrides.get(current));
            if !parents {
                break;
            }
            let Some((parent, _)) = current.rsplit_once('/') else {
                break;
            };
            current = parent;
        }
        if !result.into_iter().all(f32::is_finite) {
            return Err(format!("模型层 {layer} 挂接点变换溢出"));
        }
        Ok(result)
    }
    /// 原生 copyFrom 所需的实际部件变换；可见性由护甲槽位自身决定。
    pub fn resolved_pose(
        &self,
        layer: &str,
        path: &str,
        overrides: &BTreeMap<String, PartPose>,
    ) -> Result<PartPose, String> {
        let parts = self
            .layers
            .get(layer)
            .ok_or_else(|| format!("实体模型层不存在：{layer}"))?;
        validate_poses(layer, parts, overrides)?;
        let part = parts
            .get(path)
            .ok_or_else(|| format!("模型层 {layer} 未声明部件 {path}"))?;
        let pose = overrides.get(path);
        Ok(PartPose {
            pivot: Some(pose.and_then(|p| p.pivot).unwrap_or(part.pivot)),
            rotation: Some(pose.and_then(|p| p.rotation).unwrap_or(part.rotation)),
            scale: Some(pose.and_then(|p| p.scale).unwrap_or(part.scale)),
            visible: None,
            skip_draw: None,
        })
    }
    /// 以完整父链变换源角点；不复制裁剪后的方块模型，不删除零厚度原版面。
    pub fn bake(
        &self,
        layer: &str,
        overrides: &BTreeMap<String, PartPose>,
    ) -> Result<Vec<ModelQuad>, String> {
        self.bake_deformed(layer, overrides, 0.)
    }
    /// 在真实 Cuboid 的局部空间应用旧版 ModelBiped 的像素膨胀，再做同一父链变换。
    /// 零厚度且已丢失两侧身份的数据无法恢复膨胀，明确报错，不臆造体积。
    pub fn bake_deformed(
        &self,
        layer: &str,
        overrides: &BTreeMap<String, PartPose>,
        inflate_pixels: f32,
    ) -> Result<Vec<ModelQuad>, String> {
        if !inflate_pixels.is_finite() || inflate_pixels < 0. {
            return Err("模型膨胀必须为非负有限数".into());
        }
        let inflate = inflate_pixels / 16.;
        let parts = self
            .layers
            .get(layer)
            .ok_or_else(|| format!("实体模型层不存在：{layer}"))?;
        validate_poses(layer, parts, overrides)?;
        let mut output = Vec::new();
        for (path, part) in parts.iter() {
            let mut chain = Vec::new();
            let mut current = path.as_str();
            loop {
                let source = &parts[current];
                let pose = overrides.get(current);
                if !pose.and_then(|p| p.visible).unwrap_or(source.visible) {
                    chain.clear();
                    break;
                }
                chain.push((source, pose));
                let Some((parent, _)) = current.rsplit_once('/') else {
                    break;
                };
                current = parent;
            }
            if chain.is_empty()
                || overrides
                    .get(path)
                    .and_then(|p| p.skip_draw)
                    .unwrap_or(part.skip_draw)
            {
                continue;
            }
            for (cube, vertices) in &part.cubes {
                let bounds = if inflate != 0. && !vertices.is_empty() {
                    let lo = std::array::from_fn::<_, 3, _>(|i| {
                        vertices.iter().map(|v| v[i]).fold(f32::INFINITY, f32::min)
                    });
                    let hi = std::array::from_fn::<_, 3, _>(|i| {
                        vertices
                            .iter()
                            .map(|v| v[i])
                            .fold(f32::NEG_INFINITY, f32::max)
                    });
                    if (0..3).any(|i| lo[i] == hi[i]) {
                        return Err(format!(
                            "模型 {layer} {path}/{cube} 的退化立方体无法从四角恢复膨胀"
                        ));
                    }
                    Some((lo, hi))
                } else {
                    None
                };
                for face in vertices.chunks_exact(4) {
                    let mut corners = [[0.; 5]; 4];
                    for (destination, source) in corners.iter_mut().zip(face) {
                        let mut xyz = [source[0], source[1], source[2]];
                        if let Some((lo, hi)) = bounds {
                            for i in 0..3 {
                                if xyz[i] == lo[i] {
                                    xyz[i] -= inflate;
                                } else if xyz[i] == hi[i] {
                                    xyz[i] += inflate;
                                }
                            }
                        }

                        for (part, pose) in &chain {
                            xyz = part_point(xyz, part, *pose);
                        }
                        if !xyz.into_iter().all(f32::is_finite) {
                            return Err(format!("模型层 {layer} 变换后溢出"));
                        }
                        *destination = [xyz[0], xyz[1], xyz[2], source[3], source[4]];
                    }
                    output.push(ModelQuad {
                        part: path.clone(),
                        cube: *cube,
                        corners,
                    });
                }
            }
        }
        Ok(output)
    }
}

/// ModelPart 的 Quaternion.rotationZYX：角点依次应用 X、Y、Z，再加像素 pivot。
fn part_point(mut xyz: [f32; 3], part: &Part, pose: Option<&PartPose>) -> [f32; 3] {
    let scale = pose.and_then(|p| p.scale).unwrap_or(part.scale);
    let rotation = pose.and_then(|p| p.rotation).unwrap_or(part.rotation);
    let pivot = pose.and_then(|p| p.pivot).unwrap_or(part.pivot);
    for axis in 0..3 {
        xyz[axis] *= scale[axis];
    }
    // ModelPart 的 Quaternion.rotationZYX 对角点依次应用 X、Y、Z。
    for (axis, angle) in rotation.into_iter().enumerate() {
        let (s, c) = angle.sin_cos();
        xyz = match axis {
            0 => [xyz[0], c * xyz[1] - s * xyz[2], s * xyz[1] + c * xyz[2]],
            1 => [c * xyz[0] + s * xyz[2], xyz[1], -s * xyz[0] + c * xyz[2]],
            _ => [c * xyz[0] - s * xyz[1], s * xyz[0] + c * xyz[1], xyz[2]],
        };
    }
    for axis in 0..3 {
        xyz[axis] += pivot[axis] / 16.;
    }
    xyz
}
fn validate_poses(
    layer: &str,
    parts: &BTreeMap<String, Part>,
    overrides: &BTreeMap<String, PartPose>,
) -> Result<(), String> {
    for (path, pose) in overrides {
        if !parts.contains_key(path) {
            return Err(format!("模型层 {layer} 未声明部件 {path}"));
        }
        if !pose
            .pivot
            .iter()
            .chain(pose.rotation.iter())
            .chain(pose.scale.iter())
            .flatten()
            .all(|n| n.is_finite())
        {
            return Err(format!("部件姿态 {path} 含非有限数"));
        }
    }
    Ok(())
}
