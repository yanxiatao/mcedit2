//! 世界文件与未保存地图共用的只读解码；仅使用已核对版本的原版 MapColor。
use crate::nbt_document::{Document, Value};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// 一份源地图及其内容身份，保留客户端实际使用的中心和持久化装饰。
#[derive(Clone, Debug)]
pub struct MapImage {
    pub id: i32,
    pub source: PathBuf,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub dimension: String,
    pub scale: i8,
    pub center: [i32; 2],
    pub unlimited_tracking: bool,
    pub decorations: Vec<crate::map_decorations::Decoration>,
    pub rgba: Vec<u8>,
    pub warnings: Vec<String>,
}
impl MapImage {
    /// 资源 ID 包含内容摘要，使重读后的新图像不会复用旧 UV。
    pub fn texture_id(&self) -> String {
        format!("mcedit2:world-map/{}/{}", self.id, self.sha256)
    }
}
const COLORS: [u32; 62] = [
    0, 8368696, 16247203, 13092807, 16711680, 10526975, 10987431, 31744, 16777215, 10791096,
    9923917, 7368816, 4210943, 9402184, 16776437, 14188339, 11685080, 6724056, 15066419, 8375321,
    15892389, 5000268, 10066329, 5013401, 8339378, 3361970, 6704179, 6717235, 10040115, 1644825,
    16445005, 6085589, 4882687, 55610, 8476209, 7340544, 13742497, 10441252, 9787244, 7367818,
    12223780, 6780213, 10505550, 3746083, 8874850, 5725276, 8014168, 4996700, 4993571, 5001770,
    9321518, 2430480, 12398641, 9715553, 6035741, 1474182, 3837580, 5647422, 1356933, 6579300,
    14200723, 8365974,
];
/// Java 1.12.2、1.20.1 和 1.21.1 原版地图色表；透明色返回 alpha=0。
pub fn color_for_version(version: i32, code: u8) -> Option<[u8; 4]> {
    let limit = match version {
        1343 => 52,
        3465 | 3955 => 62,
        _ => return None,
    };
    let index = (code >> 2) as usize;
    if index >= limit {
        return None;
    }
    let shade = [180u32, 220, 255, 135][(code & 3) as usize];
    let color = *COLORS.get(index)?;
    if index == 0 {
        return Some([0, 0, 0, 0]);
    }
    Some([
        ((color >> 16 & 255) * shade / 255) as u8,
        ((color >> 8 & 255) * shade / 255) as u8,
        ((color & 255) * shade / 255) as u8,
        255,
    ])
}
/// 读取 Java 世界的 data/map_ID.dat；绝不跟随地图文件或 data 目录符号链接。
pub fn read(root: &Path, id: i32, data_version: Option<i32>) -> Result<MapImage, String> {
    if id < 0 {
        return Err(format!("地图 ID {id} 为负数"));
    }
    let dir = root.join("data");
    let metadata = fs::symlink_metadata(&dir).map_err(|e| format!("地图目录无法读取：{e}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("地图目录不是普通目录".into());
    }
    let source = dir.join(format!("map_{id}.dat"));
    let meta = fs::symlink_metadata(&source).map_err(|e| format!("地图 {id} 无法读取：{e}"))?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 16 * 1024 * 1024 {
        return Err(format!("地图 {id} 文件类型或大小无效"));
    }
    let data = fs::read(&source).map_err(|e| format!("地图 {id} 读取失败：{e}"))?;
    from_bytes(source, id, data_version, &data)
}
/// 解码真实地图载荷；内存候选与磁盘文件保留同一内容摘要和目标路径。
pub fn from_bytes(
    source: PathBuf,
    id: i32,
    data_version: Option<i32>,
    data: &[u8],
) -> Result<MapImage, String> {
    let version = data_version
        .filter(|v| matches!(*v, 1343 | 3465 | 3955))
        .ok_or("未知地图版本，拒绝猜测色表")?;
    if id < 0 || data.len() > 16 * 1024 * 1024 {
        return Err("地图 ID 或载荷大小无效".into());
    }
    let sha256 = format!("{:x}", Sha256::digest(data));
    let mut raw = Vec::new();
    if data.starts_with(&[0x1f, 0x8b]) {
        GzDecoder::new(data)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut raw)
            .map_err(|e| format!("地图 {id} gzip 无效：{e}"))?;
        if raw.len() > 16 * 1024 * 1024 {
            return Err(format!("地图 {id} 解压过大"));
        }
    } else {
        raw.extend_from_slice(data);
    }
    let doc = Document::new(raw).map_err(|e| format!("地图 {id} NBT 无效：{e}"))?;
    match doc.inspect("/DataVersion") {
        Ok(Value::Int(v)) if v == version => (),
        Err(_) if version == 1343 => (),
        Ok(Value::Int(v)) => return Err(format!("地图 {id} DataVersion {v} 与世界不一致")),
        _ => return Err(format!("地图 {id} DataVersion 缺失或类型无效")),
    };
    let dimension = match doc.inspect("/data/dimension") {
        Ok(Value::String(v)) if version != 1343 && !v.is_empty() => v,
        Ok(Value::Byte(v)) if version == 1343 => match v {
            -1 => "minecraft:the_nether".into(),
            0 => "minecraft:overworld".into(),
            1 => "minecraft:the_end".into(),
            _ => return Err(format!("地图 {id} 旧维度 {v} 未确认")),
        },
        _ => return Err(format!("地图 {id} 缺少有效维度")),
    };
    let scale = match doc.inspect("/data/scale") {
        Ok(Value::Byte(v)) if (0..=4).contains(&v) => v,
        _ => return Err(format!("地图 {id} 缩放级别无效")),
    };
    let colors = match doc
        .inspect("/data/colors")
        .map_err(|e| format!("地图 {id} colors 缺失：{e}"))?
    {
        Value::ByteArray(v) => v,
        _ => return Err(format!("地图 {id} colors 不是 ByteArray")),
    };
    let read_size = |path: &str| -> Result<usize, String> {
        match doc.inspect(path) {
            Ok(Value::Short(n)) if n > 0 => Ok(n as usize),
            Ok(Value::Int(n)) if n > 0 => Ok(n as usize),
            Err(_) => Ok(128),
            _ => Err(format!("地图 {id} 尺寸无效")),
        }
    };
    let width = read_size("/data/width")?;
    let height = read_size("/data/height")?;
    let count = width
        .checked_mul(height)
        .ok_or_else(|| format!("地图 {id} 尺寸溢出"))?;
    if colors.len() != count || (version != 1343 && (width != 128 || height != 128)) {
        return Err(format!("地图 {id} 尺寸或颜色数量无效"));
    }
    let mut rgba = Vec::with_capacity(colors.len() * 4);
    let mut unknown = 0usize;
    for value in colors {
        match color_for_version(version, value as u8) {
            Some(pixel) => rgba.extend(pixel),
            None => {
                unknown += 1;
                rgba.extend([0, 0, 0, 0]);
            }
        }
    }
    if unknown > 0 {
        return Err(format!(
            "地图 {id} 有 {unknown} 个此版本未分配色号，拒绝猜测颜色"
        ));
    }
    let data = doc.inspect("/data").map_err(|e| e.to_string())?;
    let Value::Compound(fields) = &data else {
        return Err("地图 data 必须为 Compound".into());
    };
    let center = ["xCenter", "zCenter"].map(|name| {
        match fields.iter().find(|f| f.name == name).map(|f| &f.value) {
            Some(Value::Int(n)) => Ok(*n),
            None => Ok(0), // 原版 getInt 缺失时的中心。
            _ => Err(format!("地图 {name} 必须为 Int")),
        }
    });
    let [x, z] = center;
    let center = [x?, z?];
    let unlimited_tracking = match fields
        .iter()
        .find(|f| f.name == "unlimitedTracking")
        .map(|f| &f.value)
    {
        None | Some(Value::Byte(0)) => false,
        Some(Value::Byte(1)) => true,
        _ => return Err("地图 unlimitedTracking 必须为 Bool".into()),
    };
    let decorations = crate::map_decorations::saved(&data, version, center, scale)?;
    Ok(MapImage {
        id,
        source,
        sha256,
        width: u32::try_from(width).map_err(|_| format!("地图 {id} 宽度溢出"))?,
        height: u32::try_from(height).map_err(|_| format!("地图 {id} 高度溢出"))?,
        dimension,
        scale,
        center,
        unlimited_tracking,
        decorations,
        rgba,
        warnings: Vec::new(),
    })
}
