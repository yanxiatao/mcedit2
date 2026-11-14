//! 按功能类别执行整组行为验证；新增检查优先放入现有类别。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Category {
    Formats,
    Geometry,
    Interchange,
    Recovery,
}

/// 原始 Java 格式、NBT、区段和启动输入
#[test]
fn java_formats_and_source_identity() {
    crate::run_test_category(Category::Formats);
}

/// 源查询、完整高度和远景采样
#[test]
fn source_queries_and_geometry() {
    crate::run_test_category(Category::Geometry);
}

/// 结构及剪贴板格式编解码
#[test]
fn schematic_and_clipboard_formats() {
    crate::run_test_category(Category::Interchange);
}

/// Region 计划、完整性和恢复状态机
#[test]
fn region_plans_and_recovery() {
    crate::run_test_category(Category::Recovery);
}
