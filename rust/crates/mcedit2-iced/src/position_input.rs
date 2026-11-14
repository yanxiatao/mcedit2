//! 放置坐标的显示模式和整数换算；相对基准来自实际选区或原稿，独立于结构 offset。
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element, Length,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mode {
    #[default]
    Absolute,
    Relative([i32; 3]),
}
fn format(point: [i64; 3]) -> String {
    format!("{}, {}, {}", point[0], point[1], point[2])
}
impl Mode {
    fn decode(self, input: &str) -> Result<[i32; 3], String> {
        let origin = match self {
            Self::Absolute => {
                let (x, y, z) = mcedit2_world::block_lookup::parse_xyz(input)
                    .map_err(|error| error.to_string())?;
                return Ok([x, y, z]);
            }
            Self::Relative(origin) => origin,
        };
        let values: [i64; 3] = input
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<i64>()
                    .map_err(|_| "相对坐标必须为三个逗号分隔的整数".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "相对坐标必须有三个分量")?;
        let mut point = [0; 3];
        for axis in 0..3 {
            point[axis] = values[axis]
                .checked_add(i64::from(origin[axis]))
                .and_then(|value| i32::try_from(value).ok())
                .ok_or("相对坐标换算后的目标超出整数范围")?;
        }
        Ok(point)
    }
    pub(crate) fn resolve(self, input: &str, origin: Option<[i32; 3]>) -> Result<[i32; 3], String> {
        if let Self::Relative(bound) = self {
            if origin != Some(bound) {
                return Err("相对坐标的来源基准已改变，请采用当前来源基准或切换为绝对坐标".into());
            }
        }
        self.decode(input)
    }
    pub(crate) fn encode(self, point: [i32; 3]) -> String {
        let origin = match self {
            Self::Absolute => [0; 3],
            Self::Relative(origin) => origin,
        };
        format(std::array::from_fn(|axis| {
            i64::from(point[axis]) - i64::from(origin[axis])
        }))
    }
}
#[derive(Debug, Clone)]
enum Edit {
    Input(String),
    Relative(bool),
}
#[derive(Debug, Clone)]
pub(crate) struct Request {
    mode: Mode,
    origin: Option<[i32; 3]>,
    edit: Edit,
}
impl Request {
    pub(crate) fn typing(&self) -> bool {
        matches!(self.edit, Edit::Input(_))
    }
    pub(crate) fn apply(
        self,
        mode: Mode,
        input: &str,
        origin: Option<[i32; 3]>,
    ) -> Result<(Mode, String), String> {
        if self.mode != mode || self.origin != origin {
            return Err("坐标输入对应的显示模式或来源基准已改变，请使用当前控件".into());
        }
        match self.edit {
            Edit::Input(value) => Ok((mode, value)),
            Edit::Relative(relative) => {
                let point = mode.decode(input)?;
                let next = if relative {
                    Mode::Relative(origin.ok_or("没有实际选区或原稿，不能启用相对坐标")?)
                } else {
                    Mode::Absolute
                };
                Ok((next, next.encode(point)))
            }
        }
    }
}
pub(crate) fn view<'a>(
    input: &'a str,
    mode: Mode,
    origin: Option<[i32; 3]>,
    editable: bool,
) -> Element<'a, Request> {
    let make = move |edit| Request { mode, origin, edit };
    let relative = matches!(mode, Mode::Relative(_));
    let mut body = column![
        text_input(
            if relative {
                "相对位移 X,Y,Z"
            } else {
                "目标坐标 X,Y,Z"
            },
            input
        )
        .on_input_maybe(editable.then_some(move |value| make(Edit::Input(value)))),
        checkbox(relative).label("相对来源坐标").on_toggle_maybe(
            (editable && (relative || origin.is_some()))
                .then_some(move |value| make(Edit::Relative(value)))
        ),
    ]
    .spacing(4)
    .width(Length::Fill);
    if let Mode::Relative(bound) = mode {
        body = body.push(
            text(format!(
                "相对基准：{}, {}, {}",
                bound[0], bound[1], bound[2]
            ))
            .size(12),
        );
        if origin != Some(bound) {
            body = body.push(
                row![
                    text("来源基准已改变；换算基准会保持实际目标位置。"),
                    button("采用当前来源基准").on_press_maybe(
                        (editable && origin.is_some()).then_some(make(Edit::Relative(true)))
                    ),
                ]
                .spacing(5)
                .wrap(),
            );
        } else {
            body = body.push(match mode.resolve(input, origin) {
                Ok(point) => text(format!(
                    "实际目标：{}, {}, {}",
                    point[0], point[1], point[2]
                ))
                .size(12),
                Err(error) => text(error).size(12),
            });
        }
    }
    body.into()
}
