//! 显示帧的统一尺寸：布局/射线使用逻辑坐标，纹理导数使用实际栅格像素。
use iced::Size;
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RenderViewport {
    pub logical: Size,
    pub raster: Size<u32>,
}
impl RenderViewport {
    /// 目标纹理不允许空维度；超过设备限制时统一缩小两轴，保持显示比例。
    pub fn at_scale(logical: Size, scale: f32, maximum: u32) -> Self {
        assert!(
            logical.width.is_finite()
                && logical.height.is_finite()
                && logical.width >= 0.
                && logical.height >= 0.,
            "显示布局尺寸无效"
        );
        assert!(
            scale.is_finite() && scale > 0. && maximum > 0,
            "显示缩放或栅格设备限制无效"
        );
        let width = f64::from((logical.width * scale).max(1.));
        let height = f64::from((logical.height * scale).max(1.));
        assert!(width.is_finite() && height.is_finite(), "显示像素尺寸溢出");
        let fit = (f64::from(maximum) / width.max(height)).min(1.);
        Self {
            logical,
            raster: Size::new(
                (width * fit).ceil().min(f64::from(maximum)) as u32,
                (height * fit).ceil().min(f64::from(maximum)) as u32,
            ),
        }
    }
    /// 软件光栅器和无 DPI 缩放的离屏目标逐逻辑像素绘制。
    pub fn one_to_one(logical: Size) -> Self {
        Self::at_scale(logical, 1., u32::MAX)
    }
    pub fn raster_size(self) -> Size {
        Size::new(self.raster.width as f32, self.raster.height as f32)
    }
}
