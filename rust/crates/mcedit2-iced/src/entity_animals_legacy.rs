//! 旧马的保存姿态、分组幼年变换及原生 LayeredTexture；不执行未保存的客户端动画。
use super::*;
use crate::textures::VisibilityIdentity;
use sha2::{Digest, Sha256};

pub(super) fn horse(id: &str) -> bool {
    matches!(
        id,
        "minecraft:horse"
            | "minecraft:donkey"
            | "minecraft:mule"
            | "minecraft:zombie_horse"
            | "minecraft:skeleton_horse"
    )
}
pub(super) fn read(id: &str, visual: &EntityVisual, source: &Source) -> Result<Animal, String> {
    let [yaw, pitch] = visual.rotation.ok_or("旧马缺少实际 Rotation")?;
    let young = int(visual, "Age", 0)? < 0;
    let eating = byte(visual, "EatingHaystack")?;
    let donkey = matches!(id, "minecraft:donkey" | "minecraft:mule");
    let mut animal = Animal {
        layer: match id {
            "minecraft:horse" => "minecraft:horse#main",
            "minecraft:donkey" => "minecraft:donkey#main",
            "minecraft:mule" => "minecraft:mule#main",
            "minecraft:zombie_horse" => "minecraft:zombie_horse#main",
            "minecraft:skeleton_horse" => "minecraft:skeleton_horse#main",
            _ => unreachable!("旧马模型已绑定"),
        },
        poses: BTreeMap::new(),
        family: Family::LegacyHorse,
        young,
        outer: match id {
            "minecraft:donkey" => 0.87,
            "minecraft:mule" => 0.92,
            _ => 1.,
        },
        yaw,
        tame: false,
        angry: false,
        eating,
    };
    let p = &mut animal.poses;
    let head_pitch = if eating {
        2.1816616
    } else {
        0.5235988 + pitch.to_radians()
    };
    let head_y = if eating { 11. } else { 4. };
    for path in [
        "/head",
        "/neck",
        "/mane",
        "/horse_left_ear",
        "/horse_right_ear",
        "/donkey_left_ear",
        "/donkey_right_ear",
        "/head_saddle",
        "/left_saddle_mouth",
        "/right_saddle_mouth",
    ] {
        let roll = match path {
            "/donkey_left_ear" => 0.2617994,
            "/donkey_right_ear" => -0.2617994,
            _ => 0.,
        };
        pose(
            p,
            path,
            Some([0., head_y, -10.]),
            Some([head_pitch, 0., roll]),
        );
    }
    for path in ["/left_saddle_line", "/right_saddle_line"] {
        pose(
            p,
            path,
            Some([0., head_y, -10.]),
            Some([pitch.to_radians(), 0., 0.]),
        );
    }
    for path in ["/horse_left_ear", "/horse_right_ear"] {
        visible(p, path, !donkey);
    }
    for path in ["/donkey_left_ear", "/donkey_right_ear"] {
        visible(p, path, donkey);
    }
    let saddle = !young
        && source
            .items
            .get(&Slot::Saddle)
            .is_some_and(|item| item.id == "minecraft:saddle");
    let riding = saddle && passengers(visual)?;
    for path in [
        "/saddle_bottom",
        "/saddle_front",
        "/saddle_back",
        "/left_stirrup",
        "/left_stirrup_leather",
        "/right_stirrup",
        "/right_stirrup_leather",
        "/head_saddle",
        "/left_saddle_mouth",
        "/right_saddle_mouth",
    ] {
        visible(p, path, saddle);
    }
    for path in ["/left_saddle_line", "/right_saddle_line"] {
        visible(p, path, riding);
    }
    for path in [
        "/left_stirrup",
        "/left_stirrup_leather",
        "/right_stirrup",
        "/right_stirrup_leather",
    ] {
        pose(
            p,
            path,
            None,
            Some([if riding { -1.0471976 } else { 0. }, 0., 0.]),
        );
    }
    for path in ["/left_chest", "/right_chest"] {
        visible(p, path, !young && donkey && byte(visual, "ChestedHorse")?);
    }
    Ok(animal)
}
pub(super) fn young(part: &str, eating: bool) -> ([f32; 3], [f32; 3]) {
    if part == "/head" || part.starts_with("/head/") || part.ends_with("_ear") {
        (
            [0.625; 3],
            [
                0.,
                if eating { 0.45 } else { 0.675 },
                if eating { 0.075 } else { 0. },
            ],
        )
    } else if part.ends_with("_leg") || part.ends_with("_shin") || part.ends_with("_hoof") {
        ([0.5, 0.75, 0.5], [0., 0.475, 0.])
    } else {
        ([0.5; 3], [0., 0.675, 0.])
    }
}

struct Entry {
    identity: VisibilityIdentity,
    sources: Vec<String>,
    texture: Arc<RuntimeTexture>,
    bytes: usize,
}
#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}
/// cdk 创建首图大小的透明 ARGB 画布，按来源顺序在 (0,0) SrcOver，不缩放。
fn texture(
    sources: &[String],
    bank: &TextureBank,
    stop: &dyn Fn() -> bool,
) -> Result<Arc<RuntimeTexture>, String> {
    let identity = bank.visibility_id();
    {
        let mut cache = cache().lock().expect("旧马材质缓存锁被毒化");
        if let Some(index) = cache
            .entries
            .iter()
            .position(|e| e.identity == identity && e.sources == sources)
        {
            let entry = cache.entries.remove(index).expect("已命中的旧马材质存在");
            let texture = Arc::clone(&entry.texture);
            cache.entries.push_back(entry);
            return Ok(texture);
        }
    }
    let base = bank
        .simple_image(&sources[0])
        .ok_or_else(|| format!("旧马整图缺失：{}", sources[0]))?;
    let (width, height) = (base.width, base.height);
    if base.rgba.len() as u64 > bank.atlas().ok_or("旧马缺少图集")?.remaining_bytes() {
        return Err("旧马合成图超出剩余资源预算".into());
    }
    let mut rgba = vec![0; base.rgba.len()];
    for source in sources {
        let image = bank
            .simple_image(source)
            .ok_or_else(|| format!("旧马整图缺失：{source}"))?;
        for y in 0..height.min(image.height) {
            if y.is_multiple_of(64) && stop() {
                return Err("旧马材质准备已取消".into());
            }
            for x in 0..width.min(image.width) {
                let source = ((y as usize) * (image.width as usize) + x as usize) * 4;
                let at = ((y as usize) * (width as usize) + x as usize) * 4;
                let input = &image.rgba[source..source + 4];
                let output = &mut rgba[at..at + 4];
                let alpha = u32::from(input[3]);
                let old = u32::from(output[3]);
                let combined = alpha * 255 + old * (255 - alpha);
                for channel in 0..3 {
                    output[channel] = if combined == 0 {
                        0
                    } else {
                        ((u32::from(input[channel]) * alpha * 255
                            + u32::from(output[channel]) * old * (255 - alpha)
                            + combined / 2)
                            / combined) as u8
                    };
                }
                output[3] = ((combined + 127) / 255) as u8;
            }
        }
    }
    if stop() {
        return Err("旧马材质准备已取消".into());
    }
    let mut hash = Sha256::new();
    hash.update(width.to_le_bytes());
    hash.update(height.to_le_bytes());
    hash.update(&rgba);
    let texture = Arc::new(RuntimeTexture {
        id: format!("mcedit2:runtime/legacy-horse/{:x}", hash.finalize()),
        width,
        height,
        rgba,
        sampler: Some(Default::default()),
    });
    let bytes = texture.rgba.len()
        + std::mem::size_of::<Entry>()
        + sources
            .iter()
            .map(|s| s.len() + std::mem::size_of::<String>())
            .sum::<usize>();
    const MAX_BYTES: usize = 8 * 1024 * 1024;
    if bytes <= MAX_BYTES {
        let mut cache = cache().lock().expect("旧马材质缓存锁被毒化");
        while cache.bytes + bytes > MAX_BYTES {
            let entry = cache.entries.pop_front().expect("已有旧马缓存可释放");
            cache.bytes -= entry.bytes;
        }
        cache.bytes += bytes;
        cache.entries.push_back(Entry {
            identity,
            sources: sources.to_vec(),
            texture: Arc::clone(&texture),
            bytes,
        });
    }
    Ok(texture)
}
pub(super) fn prepare(
    mut result: Prepared,
    animal: &Animal,
    catalog: &Catalog,
    visual: &EntityVisual,
    source: &Source,
    id: &str,
    bank: &TextureBank,
    stop: &dyn Fn() -> bool,
) -> Result<Prepared, String> {
    // 旧 RenderHorse 的盔甲属于身体合成图，隐形时没有独立盔甲层。
    if byte(visual, "Invisible")? {
        return Ok(result);
    }
    let texture = if id == "minecraft:horse" {
        let variant = int(visual, "Variant", 0)?;
        let color = [
            "white",
            "creamy",
            "chestnut",
            "brown",
            "black",
            "gray",
            "darkbrown",
        ][(variant as u32 & 255) as usize % 7];
        let mut images = vec![format!("minecraft:entity/horse/horse_{color}")];
        let marking = [
            None,
            Some("white"),
            Some("whitefield"),
            Some("whitedots"),
            Some("blackdots"),
        ][((variant as u32 >> 8) & 255) as usize % 5];
        if let Some(marking) = marking {
            images.push(format!("minecraft:entity/horse/horse_markings_{marking}"));
        }
        if let Some(item) = source.items.get(&Slot::Body) {
            known_stack(item, 1343)?;
            let material = match item.id.as_str() {
                "minecraft:iron_horse_armor" => Some("iron"),
                "minecraft:golden_horse_armor" => Some("gold"),
                "minecraft:diamond_horse_armor" => Some("diamond"),
                item if item.starts_with("minecraft:") => None,
                item => return Err(format!("旧马铠 {item} 缺少真实原生定义")),
            };
            if let Some(material) = material {
                images.push(format!(
                    "minecraft:entity/horse/armor/horse_armor_{material}"
                ));
            }
        }
        let image = texture(&images, bank, stop)?;
        let id = image.id.clone();
        result.textures.insert(id.clone(), image);
        id
    } else {
        match id {
            "minecraft:donkey" => "minecraft:entity/horse/donkey",
            "minecraft:mule" => "minecraft:entity/horse/mule",
            "minecraft:skeleton_horse" => "minecraft:entity/horse/horse_skeleton",
            "minecraft:zombie_horse" => "minecraft:entity/horse/horse_zombie",
            _ => unreachable!("旧马模型已绑定"),
        }
        .into()
    };
    append_model(
        &mut result,
        animal,
        catalog,
        animal.layer,
        &texture,
        [1.; 3],
        false,
    )?;
    Ok(result)
}
