//! 静态材质数据类型；glTF 解码实验仅在回归验证中编译，未接入 Java 资源发现。
#[derive(Clone, Debug)]
pub(crate) struct StaticPbrMaterial {
    pub base_color: [f32; 4],
    #[cfg(test)]
    pub metallic: f32,
    #[cfg(test)]
    pub roughness: f32,
    #[cfg(test)]
    pub emissive: [f32; 3],
    pub alpha_mode: String,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    #[cfg(test)]
    pub base_color_uri: Option<String>,
    pub base_color_texture: Option<String>,
}
