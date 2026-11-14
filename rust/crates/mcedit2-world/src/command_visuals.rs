//! 命令方块的静态只读几何；不执行命令，也不猜测选择器或实体位置。
#[path = "command_arguments.rs"]
mod arguments;
#[derive(Clone, Debug)]
pub struct BoxVisual {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub color: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct ArcVisual {
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub color: [f32; 4],
}
#[derive(Clone, Debug)]
pub struct Visuals {
    pub dimension: Option<String>,
    pub boxes: Vec<BoxVisual>,
    pub arcs: Vec<ArcVisual>,
    pub diagnostics: Vec<String>,
}
const POS: [f32; 4] = [0.2, 0.9, 0.7, 0.6];
const AREA: [f32; 4] = [0.9, 0.6, 0.3, 0.6];
const EXEC: [f32; 4] = [0.9, 0.2, 0.2, 0.6];
fn coord(s: &str, base: f64) -> Result<f64, String> {
    if s.starts_with('^') {
        return Err("局部坐标依赖执行朝向，无法静态求值".into());
    }
    let (relative, body) = s.strip_prefix('~').map_or((false, s), |x| (true, x));
    let number = if body.is_empty() && relative {
        0.
    } else {
        body.parse::<f64>().map_err(|_| format!("坐标无效：{s}"))?
    };
    let value = if relative { base + number } else { number };
    if !value.is_finite() || value.abs() > 30_000_000. {
        return Err(format!("坐标超出世界边界：{s}"));
    }
    Ok(value)
}
fn point(args: &[&str], i: usize, base: [f64; 3]) -> Result<[f64; 3], String> {
    if args.len() < i + 3 {
        return Err("命令缺少坐标".into());
    }
    Ok([
        coord(args[i], base[0])?,
        coord(args[i + 1], base[1])?,
        coord(args[i + 2], base[2])?,
    ])
}
fn selector(s: &str) -> Result<[f64; 3], String> {
    let spec = s
        .strip_prefix('@')
        .and_then(|s| s.split_once('['))
        .and_then(|(_, s)| s.strip_suffix(']'))
        .ok_or("选择器没有静态 xyz，无法确定执行位置")?;
    let mut values = [None; 3];
    for arg in arguments::split(spec, true)? {
        let Some((name, value)) = arg.split_once('=') else {
            return Err("选择器包含无法解析的参数".into());
        };
        if let Some(i) = ["x", "y", "z"].iter().position(|n| *n == name.trim()) {
            let value = value.trim();
            if values[i].is_some() || value.starts_with('~') || value.starts_with('^') {
                return Err("选择器 xyz 必须是无重复的绝对数值".into());
            }
            values[i] = Some(coord(value, 0.)?);
        }
    }
    Ok([
        values[0].ok_or("选择器缺少 x")?,
        values[1].ok_or("选择器缺少 y")?,
        values[2].ok_or("选择器缺少 z")?,
    ])
}
fn center(p: [f64; 3]) -> [f64; 3] {
    p.map(|v| v + 0.5)
}
fn add_box(out: &mut Visuals, from: [f64; 3], a: [f64; 3], b: [f64; 3], color: [f32; 4]) {
    let min = std::array::from_fn(|i| a[i].min(b[i]));
    let max = std::array::from_fn(|i| a[i].max(b[i]) + 1.);
    let mid = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
    out.boxes.push(BoxVisual { min, max, color });
    out.arcs.push(ArcVisual {
        from,
        to: mid,
        color,
    });
}
fn sound(out: &mut Visuals, args: &[&str], origin: [f64; 3], version: i32) -> Result<(), String> {
    // 三版来源的参数差异集中适配。1.20.5 起允许省略 mixer / targets。
    let optional = match version {
        1343 | 3465 => false,
        3955 => true,
        _ => {
            return Err(format!(
                "尚未核实 DataVersion {version} 的 playsound 参数格式"
            ))
        }
    };
    args.get(1).ok_or("playsound 缺少声音资源")?;
    if let Some(category) = args.get(2) {
        if ![
            "master", "music", "record", "weather", "block", "hostile", "neutral", "player",
            "ambient", "voice",
        ]
        .contains(category)
        {
            return Err(format!("playsound 声音类别无效：{category}"));
        }
    } else if !optional {
        return Err("playsound 缺少声音类别和接收玩家".into());
    }
    if !optional && args.get(3).is_none() {
        return Err("playsound 缺少接收玩家".into());
    }
    let position = if args.len() > 4 {
        point(args, 4, origin)?
    } else {
        origin
    };
    // 接收者选择器的搜索原点不等于播放位置；这里只展示命令声明的声音位置。
    add_box(out, origin, position, position, POS);
    out.diagnostics
        .push("声音方框代表播放位置；未判断接收玩家、命令条件或实际播放结果".into());
    Ok(())
}
fn execute(
    out: &mut Visuals,
    args: &[&str],
    mut origin: [f64; 3],
    version: i32,
    depth: usize,
) -> Result<(), String> {
    let mut index = 1;
    loop {
        match args
            .get(index)
            .copied()
            .ok_or("execute 缺少 run 和子命令")?
        {
            "run" => return parse(out, &args[index + 1..], origin, version, depth + 1),
            "positioned" => {
                let position = point(args, index + 1, origin)?;
                add_box(out, origin, position, position, EXEC);
                origin = position;
                index += 4;
            }
            "align" => {
                let axes = args.get(index + 1).ok_or("execute align 缺少坐标轴")?;
                let mut used = [false; 3];
                let mut position = origin;
                for axis in axes.chars() {
                    let i = match axis {
                        'x' => 0,
                        'y' => 1,
                        'z' => 2,
                        _ => return Err("execute align 只能使用 x / y / z".into()),
                    };
                    if used[i] {
                        return Err("execute align 坐标轴不能重复".into());
                    }
                    used[i] = true;
                    position[i] = position[i].floor();
                }
                add_box(out, origin, position, position, EXEC);
                origin = position;
                index += 2;
            }
            "in" => {
                let dimension = args.get(index + 1).ok_or("execute in 缺少维度")?;
                let source = out
                    .dimension
                    .as_deref()
                    .ok_or("来源文件未声明维度，不能静态确认 execute in")?;
                if *dimension != source {
                    return Err(format!("目标维度 {dimension} 与当前维度不同"));
                }
                index += 2;
            }
            other => return Err(format!("execute {other} 的执行上下文尚不能静态求值")),
        }
    }
}
fn parse(
    out: &mut Visuals,
    args: &[&str],
    origin: [f64; 3],
    version: i32,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("execute 嵌套过深".into());
    }
    let command = args.first().ok_or("命令为空")?.trim_start_matches('/');
    match command {
        "setblock" | "testforblock" | "blockdata" => {
            if args.len() < 5 {
                return Err("命令缺少方块参数".into());
            }
            let p = point(args, 1, origin)?.map(f64::floor);
            let color = match command {
                "testforblock" => [0.5, 0.2, 0.7, 0.6],
                "blockdata" => [0.2, 0.6, 0.9, 0.6],
                _ => POS,
            };
            add_box(out, origin, p, p, color);
        }
        "summon" => {
            if args.len() < 5 {
                return Err("summon 缺少实体或坐标".into());
            }
            let p = point(args, 2, origin)?;
            add_box(out, origin, p, p, [0.9, 0.2, 0.4, 0.6]);
        }
        "fill" | "clone" => {
            let a = point(args, 1, origin)?.map(f64::floor);
            let b = point(args, 4, origin)?.map(f64::floor);
            if command == "fill" {
                if args.len() < 8 {
                    return Err("fill 缺少方块参数".into());
                }
                add_box(out, origin, a, b, AREA);
            } else {
                let d = point(args, 7, origin)?.map(f64::floor);
                let min: [f64; 3] = std::array::from_fn(|i| a[i].min(b[i]));
                let max: [f64; 3] = std::array::from_fn(|i| a[i].max(b[i]));
                add_box(out, origin, a, b, [0.3, 0.5, 0.9, 0.6]);
                let end = std::array::from_fn(|i| d[i] + max[i] - min[i]);
                let src = std::array::from_fn(|i| (min[i] + max[i] + 1.) * 0.5);
                add_box(out, src, d, end, [0., 0., 0.9, 0.6]);
            }
        }
        "give" => {
            let target = *args.get(1).ok_or("缺少选择器")?;
            let p = selector(target)?;
            out.diagnostics
                .push("选择器方框代表搜索原点，不代表实际选中的实体".into());
            add_box(out, origin, p, p, POS);
        }
        "playsound" => sound(out, args, origin, version)?,
        "execute" => {
            if version >= 1451 {
                execute(out, args, origin, version, depth)?;
            } else {
                let source = selector(args.get(1).ok_or("execute 缺少目标")?)?;
                out.diagnostics
                    .push("execute 选择器方框代表搜索原点，不代表实体位置".into());
                if args
                    .get(2..5)
                    .is_none_or(|v| v.iter().any(|x| x.starts_with('~') || x.starts_with('^')))
                {
                    return Err("旧版 execute 相对坐标依赖选中实体的实际位置".into());
                }
                let p = point(args, 2, source)?;
                add_box(out, origin, source, source, EXEC);
                if p != source {
                    add_box(out, center(source), p, p, EXEC);
                }
                let rest = args.get(5..).ok_or("execute 缺少子命令")?;
                if rest.first() == Some(&"detect") {
                    return Err("execute detect 条件依赖世界状态，无法静态确认".into());
                }
                parse(out, rest, p, version, depth + 1)?;
            }
        }
        _ => return Err(format!("尚不支持静态可视化命令：{command}")),
    }
    Ok(())
}
/// 以选中命令方块位置和当前维度生成几何；错误时不返回可能误导的部分结果。
pub fn analyze(
    command: &str,
    position: [i32; 3],
    dimension: &str,
    data_version: Option<i32>,
) -> Result<Visuals, String> {
    analyze_context(command, position, Some(dimension), data_version)
}
/// 独立文件可缺少维度；普通坐标仍可解析，显式维度切换必须核对实际来源。
pub fn analyze_context(
    command: &str,
    position: [i32; 3],
    dimension: Option<&str>,
    data_version: Option<i32>,
) -> Result<Visuals, String> {
    let args = arguments::split(command, false)?;
    let mut out = Visuals {
        dimension: dimension.map(str::to_owned),
        boxes: Vec::new(),
        arcs: Vec::new(),
        diagnostics: Vec::new(),
    };
    let version = data_version.ok_or("来源缺少 DataVersion，无法确定命令源坐标语义")?;
    let origin = position.map(|p| f64::from(p) + if version >= 1451 { 0.5 } else { 0. });
    parse(&mut out, &args, origin, version, 0)?;
    Ok(out)
}
