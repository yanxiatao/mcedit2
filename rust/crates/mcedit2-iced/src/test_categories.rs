//! 按功能类别执行整组行为验证；新增检查优先放入现有类别。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Category {
    Sources,
    Assets,
    Materials,
    Models,
    SpecialModels,
    WorldGeometry,
    Rendering,
    Navigation,
    Names,
    Background,
    Hardware,
    Installed,
    Framework,
}

/// 资源来源、实例顺序与框架安装
#[test]
fn resource_sources_and_frameworks() {
    crate::run_test_category(Category::Sources);
}

/// 资源索引、加载与内存预算
#[test]
fn asset_index_loading_and_budgets() {
    crate::run_test_category(Category::Assets);
}

/// 纹理、材质、覆盖及图集
#[test]
fn texture_material_and_atlas_pipeline() {
    crate::run_test_category(Category::Materials);
}

/// 声明式模型、状态与坐标变换
#[test]
fn declarative_models_and_transforms() {
    crate::run_test_category(Category::Models);
}

/// 原版静态方块实体外形和贴图
#[test]
fn vanilla_special_block_models() {
    crate::run_test_category(Category::SpecialModels);
}

/// 完整世界建模、邻域、缓存与流式读取
#[test]
fn world_geometry_and_streaming() {
    crate::run_test_category(Category::WorldGeometry);
}

/// 投影、光栅化、透明深度与源拾取
#[test]
fn render_projection_raster_and_source_picking() {
    crate::run_test_category(Category::Rendering);
}

/// 查看导航、切片、四视图和源范围选择
#[test]
fn view_navigation_and_source_selection() {
    crate::run_test_category(Category::Navigation);
}

/// 名称语言、资源覆盖及所选方块信息
#[test]
fn localized_block_information() {
    crate::run_test_category(Category::Names);
}

/// 后台取消、过期请求和完整场景交接
#[test]
fn background_requests_and_scene_handoff() {
    crate::run_test_category(Category::Background);
}

/// 实际 GPU 的完整材质、深度、几何和像素回读
#[test]
#[ignore = "需要本地 GPU、MCEDIT2_CLIENT_JAR 和 MCEDIT2_TEST_WORLD；可设置 MCEDIT2_TEST_RESOURCE_MIB"]
fn hardware_gpu_rendering() {
    crate::run_test_category(Category::Hardware);
}

/// 本机实例的资源和完整源建模
#[test]
#[ignore = "需要 MCEDIT2_TEST_WORLD 指向包含相应模组的完整本机实例"]
fn installed_instance_resources_and_mesh() {
    crate::run_test_category(Category::Installed);
}

/// 本机准确框架版本的原生模型
#[test]
#[ignore = "需要 GPU、MCEDIT2_TEST_WORLD 和 MCEDIT2_FRAMEWORK_JAR 指向准确安装文件"]
fn installed_framework_models() {
    crate::run_test_category(Category::Framework);
}
