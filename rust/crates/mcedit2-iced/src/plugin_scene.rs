//! 已声明工具几何的视图投影；覆盖层不参与存档写入或方块拾取。
use super::*;
pub(super) fn draw(scene: &mcedit2_plugins::tool_scene::Scene, view: View, frame: &mut Frame) {
    for primitive in scene.overlay.iter().chain(&scene.cursor) {
        let points: Vec<_> = primitive
            .points
            .iter()
            .map(|point| point.map(|value| value - 0.5))
            .collect();
        if let Some(fill) = primitive.fill {
            // 穿过近裁面的面仍由下方的裁剪线框定位，不在相机后补造投影点。
            if let Some(projected) = points
                .iter()
                .map(|point| view.project(*point))
                .collect::<Option<Vec<_>>>()
            {
                let path = Path::new(|path| {
                    path.move_to(projected[0]);
                    for point in projected.iter().skip(1) {
                        path.line_to(*point);
                    }
                    path.close();
                });
                frame.fill(&path, Color::from_rgba(fill[0], fill[1], fill[2], fill[3]));
            }
        }
        if let Some(color) = primitive.stroke {
            let stroke = Stroke::default()
                .with_width(primitive.width)
                .with_color(Color::from_rgba(color[0], color[1], color[2], color[3]));
            for pair in points.windows(2) {
                if let Some((a, b)) = view.segment(pair[0], pair[1]) {
                    frame.stroke(&Path::line(a, b), stroke);
                }
            }
            if primitive.closed {
                if let Some((a, b)) = view.segment(*points.last().expect("场景已验证"), points[0])
                {
                    frame.stroke(&Path::line(a, b), stroke);
                }
            }
        }
    }
}
