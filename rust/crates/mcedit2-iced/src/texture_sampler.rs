//! SimpleTexture 的实际 texture 元数据；错误独立于图集 animation 合同传播。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Sampler {
    pub blur: bool,
    pub clamp: bool,
}
impl Sampler {
    pub const FORCE: u32 = 8;
    pub fn flags(self, force: bool) -> u32 {
        u32::from(self.blur) | (u32::from(self.clamp) << 1) | if force { Self::FORCE } else { 0 }
    }
    pub fn from_flags(flags: u32) -> Self {
        Self {
            blur: flags & 1 != 0,
            clamp: flags & 2 != 0,
        }
    }
    pub fn read(metadata: Option<&[u8]>) -> Result<Self, String> {
        let Some(metadata) = metadata else {
            return Ok(Self::default());
        };
        let document = mcedit2_world::client_json::resource_object(metadata)
            .map_err(|error| format!("整图元数据无效：{error}"))?;
        let Some(texture) = document.value.get("texture") else {
            return Ok(Self::default());
        };
        let texture = texture.as_object().ok_or("texture 必须为对象")?;
        let flag = |key| -> Result<bool, String> {
            match texture.get(key) {
                None => Ok(false),
                Some(serde_json::Value::Bool(value)) => Ok(*value),
                Some(serde_json::Value::String(value)) => Ok(value.eq_ignore_ascii_case("true")),
                Some(serde_json::Value::Number(_)) => Ok(false),
                _ => Err(format!("texture.{key} 必须为布尔原始值")),
            }
        };
        Ok(Self {
            blur: flag("blur")?,
            clamp: flag("clamp")?,
        })
    }
}
