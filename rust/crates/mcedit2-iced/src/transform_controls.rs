//! 导入、克隆 / 移动共用的原版直角与镜像操作；只生成参数，不发布世界修改。
use iced::{
    widget::{button, column, row},
    Element,
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Axis {
    X,
    Y,
    Z,
}
impl Axis {
    fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Adjustment {
    RightAngle(Axis),
    Mirror(Axis),
    ResetRotation,
    ResetScale,
}
impl Adjustment {
    pub(crate) fn rotation(self) -> bool {
        matches!(self, Self::RightAngle(_) | Self::ResetRotation)
    }
    pub(crate) fn apply(self, rotation: &str, scale: &str) -> Result<String, String> {
        fn vector(input: &str) -> Result<[f64; 3], String> {
            let values: [f64; 3] = input
                .split(',')
                .map(|part| {
                    part.trim()
                        .parse::<f64>()
                        .map_err(|_| "变换参数必须为三个逗号分隔的数字".into())
                })
                .collect::<Result<Vec<_>, String>>()?
                .try_into()
                .map_err(|_| "变换参数必须有三个分量")?;
            if values.iter().any(|value| !value.is_finite()) {
                return Err("变换参数必须为有限数字".into());
            }
            Ok(values)
        }
        let values = match self {
            Self::ResetRotation => [0.; 3],
            Self::ResetScale => [1.; 3],
            Self::RightAngle(axis) => {
                let mut values = vector(rotation)?;
                let i = axis.index();
                // 原 RotationWidget 的按钮跳到下一个直角，任意输入角度不简单加 90。
                values[i] =
                    ((values[i].rem_euclid(360.) / 90.).floor() * 90. + 90.).rem_euclid(360.);
                values
            }
            Self::Mirror(axis) => {
                let mut values = vector(scale)?;
                values[axis.index()] = -values[axis.index()];
                values
            }
        };
        Ok(format!("{}, {}, {}", values[0], values[1], values[2]))
    }
}
pub(crate) fn view(enabled: bool) -> Element<'static, Adjustment> {
    use Adjustment::*;
    use Axis::*;
    column![
        row![
            button("X 下个直角").on_press_maybe(enabled.then_some(RightAngle(X))),
            button("Y 下个直角").on_press_maybe(enabled.then_some(RightAngle(Y))),
            button("Z 下个直角").on_press_maybe(enabled.then_some(RightAngle(Z))),
            button("重设角度").on_press_maybe(enabled.then_some(ResetRotation)),
        ]
        .spacing(4)
        .wrap(),
        row![
            button("镜像 X").on_press_maybe(enabled.then_some(Mirror(X))),
            button("镜像 Y").on_press_maybe(enabled.then_some(Mirror(Y))),
            button("镜像 Z").on_press_maybe(enabled.then_some(Mirror(Z))),
            button("重设缩放").on_press_maybe(enabled.then_some(ResetScale)),
        ]
        .spacing(4)
        .wrap(),
    ]
    .spacing(4)
    .into()
}
