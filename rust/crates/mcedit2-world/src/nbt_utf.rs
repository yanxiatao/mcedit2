//! Java DataInput/DataOutput 使用的 Modified UTF-8。孤立代理项无法表示为 Rust String，明确拒绝而不替换。
use crate::WorldError;
fn invalid(s: &str) -> WorldError {
    WorldError::Invalid(s.into())
}
pub(crate) fn decode(bytes: &[u8]) -> Result<String, WorldError> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let a = bytes[i];
        if a <= 0x7f {
            units.push(a as u16);
            i += 1;
        } else if a & 0xe0 == 0xc0 {
            let b = *bytes
                .get(i + 1)
                .ok_or_else(|| invalid("NBT Modified UTF-8 末尾不完整"))?;
            if b & 0xc0 != 0x80 {
                return Err(invalid("NBT Modified UTF-8 后续字节无效"));
            }
            units.push(((a as u16 & 31) << 6) | (b as u16 & 63));
            i += 2;
        } else if a & 0xf0 == 0xe0 {
            let b = *bytes
                .get(i + 1)
                .ok_or_else(|| invalid("NBT Modified UTF-8 末尾不完整"))?;
            let c = *bytes
                .get(i + 2)
                .ok_or_else(|| invalid("NBT Modified UTF-8 末尾不完整"))?;
            if b & 0xc0 != 0x80 || c & 0xc0 != 0x80 {
                return Err(invalid("NBT Modified UTF-8 后续字节无效"));
            }
            units.push(((a as u16 & 15) << 12) | ((b as u16 & 63) << 6) | (c as u16 & 63));
            i += 3;
        } else {
            return Err(invalid("NBT Modified UTF-8 首字节无效"));
        }
    }
    String::from_utf16(&units)
        .map_err(|_| invalid("NBT 字符串包含孤立 UTF-16 代理项，当前无法编辑"))
}
pub(crate) fn encode(value: &str) -> Result<Vec<u8>, WorldError> {
    let mut bytes = Vec::new();
    for u in value.encode_utf16() {
        if (1..=0x7f).contains(&u) {
            bytes.push(u as u8);
        } else if u <= 0x7ff {
            bytes.push((0xc0 | (u >> 6)) as u8);
            bytes.push((0x80 | (u & 63)) as u8);
        } else {
            bytes.push((0xe0 | (u >> 12)) as u8);
            bytes.push((0x80 | ((u >> 6) & 63)) as u8);
            bytes.push((0x80 | (u & 63)) as u8);
        }
        if bytes.len() > 65535 {
            return Err(invalid("NBT Modified UTF-8 编码超过 65535 字节"));
        }
    }
    Ok(bytes)
}
