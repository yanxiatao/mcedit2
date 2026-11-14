//! 1.12.2 bpv 的 128×128 ModelHorse；构造参数与四角顺序来自 bpv / brq / bql。
//! 旧马的分节腿、独立头颈和鞍具不能套用现代 HorseModel。
use super::*;

fn part(uv: [f32; 2], origin: [f32; 3], size: [f32; 3], pivot: [f32; 3], inflate: f32) -> Part {
    let [x, y, z] = origin.map(|v| v - inflate);
    let end = std::array::from_fn::<_, 3, _>(|i| origin[i] + size[i] + inflate);
    let [xx, yy, zz] = end;
    let vertices = [
        [x, y, z],
        [xx, y, z],
        [xx, yy, z],
        [x, yy, z],
        [x, y, zz],
        [xx, y, zz],
        [xx, yy, zz],
        [x, yy, zz],
    ];
    let [u, v] = uv;
    let [w, h, d] = size;
    let faces = [
        ([5, 1, 2, 6], [u + d + w, v + d, u + d + w + d, v + d + h]),
        ([0, 4, 7, 3], [u, v + d, u + d, v + d + h]),
        ([5, 4, 0, 1], [u + d, v, u + d + w, v + d]),
        ([2, 3, 7, 6], [u + d + w, v + d, u + d + w + w, v]),
        ([1, 0, 3, 2], [u + d, v + d, u + d + w, v + d + h]),
        (
            [4, 5, 6, 7],
            [u + d + w + d, v + d, u + d + w + d + w, v + d + h],
        ),
    ];
    let mut corners = Vec::with_capacity(24);
    for (indices, [u1, v1, u2, v2]) in faces {
        for (index, [u, v]) in indices
            .into_iter()
            .zip([[u2, v1], [u1, v1], [u1, v2], [u2, v2]])
        {
            let [x, y, z] = vertices[index].map(|v| v / 16.);
            corners.push([x, y, z, u / 128., v / 128.]);
        }
    }
    Part {
        pivot,
        rotation: [0.; 3],
        scale: [1.; 3],
        visible: true,
        skip_draw: false,
        cubes: BTreeMap::from([(0, corners)]),
    }
}

pub(super) fn model() -> Arc<BTreeMap<String, Part>> {
    let mut parts = BTreeMap::from([(
        String::new(),
        Part {
            pivot: [0.; 3],
            rotation: [0.; 3],
            scale: [1.; 3],
            visible: true,
            skip_draw: false,
            cubes: BTreeMap::new(),
        },
    )]);
    macro_rules! add {
        ($name:literal,$uv:expr,$origin:expr,$size:expr,$pivot:expr) => {
            parts.insert(
                concat!("/", $name).into(),
                part($uv, $origin, $size, $pivot, 0.),
            );
        };
    }
    add!(
        "body",
        [0., 34.],
        [-5., -8., -19.],
        [10., 10., 24.],
        [0., 11., 9.]
    );
    add!(
        "tail_base",
        [44., 0.],
        [-1., -1., 0.],
        [2., 2., 3.],
        [0., 3., 14.]
    );
    add!(
        "tail_middle",
        [38., 7.],
        [-1.5, -2., 3.],
        [3., 4., 7.],
        [0., 3., 14.]
    );
    add!(
        "tail_tip",
        [24., 3.],
        [-1.5, -4.5, 9.],
        [3., 4., 7.],
        [0., 3., 14.]
    );
    add!(
        "left_hind_leg",
        [78., 29.],
        [-2.5, -2., -2.5],
        [4., 9., 5.],
        [4., 9., 11.]
    );
    add!(
        "left_hind_shin",
        [78., 43.],
        [-2., 0., -1.5],
        [3., 5., 3.],
        [4., 16., 11.]
    );
    add!(
        "left_hind_hoof",
        [78., 51.],
        [-2.5, 5.1, -2.],
        [4., 3., 4.],
        [4., 16., 11.]
    );
    add!(
        "right_hind_leg",
        [96., 29.],
        [-1.5, -2., -2.5],
        [4., 9., 5.],
        [-4., 9., 11.]
    );
    add!(
        "right_hind_shin",
        [96., 43.],
        [-1., 0., -1.5],
        [3., 5., 3.],
        [-4., 16., 11.]
    );
    add!(
        "right_hind_hoof",
        [96., 51.],
        [-1.5, 5.1, -2.],
        [4., 3., 4.],
        [-4., 16., 11.]
    );
    add!(
        "left_front_leg",
        [44., 29.],
        [-1.9, -1., -2.1],
        [3., 8., 4.],
        [4., 9., -8.]
    );
    add!(
        "left_front_shin",
        [44., 41.],
        [-1.9, 0., -1.6],
        [3., 5., 3.],
        [4., 16., -8.]
    );
    add!(
        "left_front_hoof",
        [44., 51.],
        [-2.4, 5.1, -2.1],
        [4., 3., 4.],
        [4., 16., -8.]
    );
    add!(
        "right_front_leg",
        [60., 29.],
        [-1.1, -1., -2.1],
        [3., 8., 4.],
        [-4., 9., -8.]
    );
    add!(
        "right_front_shin",
        [60., 41.],
        [-1.1, 0., -1.6],
        [3., 5., 3.],
        [-4., 16., -8.]
    );
    add!(
        "right_front_hoof",
        [60., 51.],
        [-1.6, 5.1, -2.1],
        [4., 3., 4.],
        [-4., 16., -8.]
    );
    add!(
        "head",
        [0., 0.],
        [-2.5, -10., -1.5],
        [5., 5., 7.],
        [0., 4., -10.]
    );
    // 两个嘴部是头部子节点；中立姿态采用 setupAnim 的相对偏移。
    add!(
        "head/upper_mouth",
        [24., 18.],
        [-2., -10., -7.],
        [4., 3., 6.],
        [0., 0.02, 0.02]
    );
    add!(
        "head/lower_mouth",
        [24., 27.],
        [-2., -7., -6.5],
        [4., 2., 5.],
        [0., 0., 0.]
    );
    add!(
        "horse_left_ear",
        [0., 0.],
        [0.45, -12., 4.],
        [2., 3., 1.],
        [0., 4., -10.]
    );
    add!(
        "horse_right_ear",
        [0., 0.],
        [-2.45, -12., 4.],
        [2., 3., 1.],
        [0., 4., -10.]
    );
    add!(
        "donkey_left_ear",
        [0., 12.],
        [-2., -16., 4.],
        [2., 7., 1.],
        [0., 4., -10.]
    );
    add!(
        "donkey_right_ear",
        [0., 12.],
        [0., -16., 4.],
        [2., 7., 1.],
        [0., 4., -10.]
    );
    add!(
        "neck",
        [0., 12.],
        [-2.05, -9.8, -2.],
        [4., 14., 8.],
        [0., 4., -10.]
    );
    add!(
        "left_chest",
        [0., 34.],
        [-3., 0., 0.],
        [8., 8., 3.],
        [-7.5, 3., 10.]
    );
    add!(
        "right_chest",
        [0., 47.],
        [-3., 0., 0.],
        [8., 8., 3.],
        [4.5, 3., 10.]
    );
    add!(
        "saddle_bottom",
        [80., 0.],
        [-5., 0., -3.],
        [10., 1., 8.],
        [0., 2., 2.]
    );
    add!(
        "saddle_front",
        [106., 9.],
        [-1.5, -1., -3.],
        [3., 1., 2.],
        [0., 2., 2.]
    );
    add!(
        "saddle_back",
        [80., 9.],
        [-4., -1., 3.],
        [8., 1., 2.],
        [0., 2., 2.]
    );
    add!(
        "left_stirrup",
        [74., 0.],
        [-0.5, 6., -1.],
        [1., 2., 2.],
        [5., 2., 2.]
    );
    add!(
        "left_stirrup_leather",
        [70., 0.],
        [-0.5, 0., -0.5],
        [1., 6., 1.],
        [5., 2., 2.]
    );
    add!(
        "right_stirrup",
        [74., 4.],
        [-0.5, 6., -1.],
        [1., 2., 2.],
        [-5., 2., 2.]
    );
    add!(
        "right_stirrup_leather",
        [80., 0.],
        [-0.5, 0., -0.5],
        [1., 6., 1.],
        [-5., 2., 2.]
    );
    add!(
        "left_saddle_mouth",
        [74., 13.],
        [1.5, -8., -4.],
        [1., 2., 2.],
        [0., 4., -10.]
    );
    add!(
        "right_saddle_mouth",
        [74., 13.],
        [-2.5, -8., -4.],
        [1., 2., 2.],
        [0., 4., -10.]
    );
    add!(
        "left_saddle_line",
        [44., 10.],
        [2.6, -6., -6.],
        [0., 3., 16.],
        [0., 4., -10.]
    );
    add!(
        "right_saddle_line",
        [44., 5.],
        [-2.6, -6., -6.],
        [0., 3., 16.],
        [0., 4., -10.]
    );
    add!(
        "mane",
        [58., 0.],
        [-1., -11.5, 5.],
        [2., 16., 4.],
        [0., 4., -10.]
    );
    parts.insert(
        "/head_saddle".into(),
        part(
            [80., 12.],
            [-2.5, -10.1, -7.],
            [5., 5., 12.],
            [0., 4., -10.],
            0.2,
        ),
    );
    for name in [
        "head",
        "horse_left_ear",
        "horse_right_ear",
        "donkey_left_ear",
        "donkey_right_ear",
        "neck",
        "mane",
        "head_saddle",
        "left_saddle_mouth",
        "right_saddle_mouth",
    ] {
        parts
            .get_mut(&format!("/{name}"))
            .expect("旧马构造部件存在")
            .rotation[0] = 0.5235988;
    }
    parts.get_mut("/donkey_left_ear").unwrap().rotation[2] = 0.2617994;
    parts.get_mut("/donkey_right_ear").unwrap().rotation[2] = -0.2617994;
    for name in ["left_chest", "right_chest"] {
        parts.get_mut(&format!("/{name}")).unwrap().rotation[1] = 1.5707964;
    }
    for (name, pitch) in [
        ("tail_base", -1.3089969),
        ("tail_middle", -1.3089969),
        ("tail_tip", -1.5707963),
    ] {
        parts.get_mut(&format!("/{name}")).unwrap().rotation[0] = pitch;
    }
    Arc::new(parts)
}
