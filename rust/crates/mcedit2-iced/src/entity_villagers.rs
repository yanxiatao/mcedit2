//! 村民与僵尸村民共用真实类型 / 职业资源，流浪商人保留独立模型与装备层。
use super::*;

/// 转化 / 完全冻结时取原生 tick=0 的静态抖动相位，身体与全部挂接点共用。
pub(super) fn zombie_yaw(
    catalog: &Catalog,
    visual: &EntityVisual,
    yaw: f32,
) -> Result<f32, String> {
    let converting = match visual.appearance.get("ConversionTime") {
        None => false,
        Some(Value::Int(time)) => *time >= 0,
        _ => return Err("僵尸村民 ConversionTime 必须为 Int".into()),
    };
    let legacy = catalog.version == "1.12.2";
    let frozen = if legacy {
        false
    } else {
        match visual.appearance.get("TicksFrozen") {
            None => false,
            Some(Value::Int(ticks)) => *ticks >= 140,
            _ => return Err("僵尸村民 TicksFrozen 必须为 Int".into()),
        }
    };
    Ok(yaw
        + if converting || frozen {
            (std::f64::consts::PI * if legacy { 0.25 } else { f64::from(0.4_f32) }) as f32
        } else {
            0.
        })
}

pub(super) fn texture_dependency(id: &str) -> bool {
    let Some((_, path)) = id.split_once(':') else {
        return false;
    };
    let Some(path) = path.strip_prefix("entity/") else {
        return false;
    };
    let Some((kind, path)) = path.split_once('/') else {
        return false;
    };
    matches!(kind, "villager" | "zombie_villager")
        && path.split_once('/').is_some_and(|(category, _)| {
            matches!(category, "type" | "profession" | "profession_level")
        })
}
fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Compound(fields) => fields
            .iter()
            .find(|field| field.name == key)
            .map(|field| &field.value),
        _ => None,
    }
}
fn resource(kind: &str, category: &str, id: &str) -> Result<String, String> {
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    if namespace.is_empty()
        || path.is_empty()
        || !namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
        })
        || !path.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_./-".contains(&byte)
        })
        || path.split('/').any(|part| part == ".." || part.is_empty())
    {
        return Err(format!("无效村民注册资源 {id}"));
    }
    Ok(format!("{namespace}:entity/{kind}/{category}/{path}"))
}
fn hat(poses: &mut BTreeMap<String, PartPose>, zombie: bool, visible: bool) {
    for path in if zombie {
        &["/head", "/hat", "/hat/hat_rim"][..]
    } else {
        &["/head"][..]
    } {
        poses.entry((*path).into()).or_default().visible = Some(visible);
    }
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
    if id == "minecraft:wandering_trader" {
        return npc::render(
            catalog,
            bank,
            rig.layer,
            "minecraft:entity/wandering_trader",
            &rig.poses,
            rig.style,
        );
    }
    let zombie = match id {
        "minecraft:villager" => false,
        "minecraft:zombie_villager" => true,
        _ => return Err(format!("{id} 没有此村民模型绑定")),
    };
    let kind = if zombie {
        "zombie_villager"
    } else {
        "villager"
    };
    if catalog.version == "1.12.2" {
        let profession = match visual.appearance.get("Profession") {
            None => 0,
            Some(Value::Int(value)) if (0..=5).contains(value) => *value as usize,
            _ => return Err("旧村民 Profession 必须为 0..5 的 Int".into()),
        };
        let name = if zombie {
            [
                "zombie_farmer",
                "zombie_librarian",
                "zombie_priest",
                "zombie_smith",
                "zombie_butcher",
                "zombie_villager",
            ][profession]
        } else {
            [
                "farmer",
                "librarian",
                "priest",
                "smith",
                "butcher",
                "villager",
            ][profession]
        };
        return npc::render(
            catalog,
            bank,
            rig.layer,
            &format!("minecraft:entity/{kind}/{name}"),
            &rig.poses,
            rig.style,
        );
    }
    let data = visual
        .appearance
        .get("VillagerData")
        .ok_or("村民缺少 VillagerData")?;
    let string = |key| match field(data, key) {
        Some(Value::String(value)) => Ok(value.as_str()),
        _ => Err(format!("VillagerData.{key} 无效")),
    };
    let ty = resource(kind, "type", string("type")?)?;
    let profession = string("profession")?;
    let prof = resource(kind, "profession", profession)?;
    let style = npc::RenderStyle {
        ordered: true,
        ..rig.style
    };
    let mut out = npc::render(
        catalog,
        bank,
        rig.layer,
        &format!("minecraft:entity/{kind}/{kind}"),
        &rig.poses,
        style,
    )?;
    let type_hat = bank.villager_hat(&ty)?;
    let profession_hat = bank.villager_hat(&prof)?;
    let mut poses = rig.poses.clone();
    hat(
        &mut poses,
        zombie,
        profession_hat == "none" || profession_hat == "partial" && type_hat != "full",
    );
    out.extend(npc::render(catalog, bank, rig.layer, &ty, &poses, style)?);
    hat(&mut poses, zombie, true);
    if !rig.young() && !matches!(profession, "minecraft:none" | "none") {
        out.extend(npc::render(catalog, bank, rig.layer, &prof, &poses, style)?);
        if !matches!(profession, "minecraft:nitwit" | "nitwit") {
            let level = match field(data, "level") {
                Some(Value::Int(level)) => *level,
                None => 1,
                _ => return Err("VillagerData.level 不是 Int".into()),
            }
            .clamp(1, 5);
            let name = ["stone", "iron", "gold", "emerald", "diamond"][(level - 1) as usize];
            out.extend(npc::render(
                catalog,
                bank,
                rig.layer,
                &format!("minecraft:entity/{kind}/profession_level/{name}"),
                &poses,
                style,
            )?);
        }
    }
    Ok(out)
}
