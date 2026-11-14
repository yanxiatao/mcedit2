//! 小地图的独立区块源、有限批次及单张材质图片；采样数据不进入编辑拾取。
use super::*;
use mcedit2_world::{
    far_view::{self, FarChunkRequest},
    surface::SurfaceView,
};

pub(super) type Tiles = BTreeMap<(i32, i32), Arc<SurfaceView>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Grid {
    pub x: i32,
    pub z: i32,
    pub width: u16,
    pub depth: u16,
    pub span: u8,
}
impl Grid {
    pub fn new(center: [f64; 3], size: Size, scale: f32) -> Result<Self, String> {
        if !center.into_iter().all(f64::is_finite)
            || !(-30_000_000. ..30_000_000.).contains(&center[0])
            || !(-30_000_000. ..30_000_000.).contains(&center[2])
        {
            return Err("小地图观察中心超出 Java 世界范围".into());
        }
        let axis = |center: f64, pixels: f32| -> Result<(i32, u16), String> {
            let half = f64::from(pixels) * f64::from(scale) * 0.5;
            let first = (center - half).floor() as i64;
            let last = (center + half).ceil() as i64;
            let start = first.div_euclid(16) * 16;
            let end = last.div_euclid(16) * 16 + 16;
            Ok((
                i32::try_from(start).map_err(|e| e.to_string())?,
                u16::try_from(end - start).map_err(|e| e.to_string())?,
            ))
        };
        let (x, width) = axis(center[0], size.width)?;
        let (z, depth) = axis(center[2], size.height)?;
        let span = if scale >= 16. {
            16
        } else if scale >= 8. {
            8
        } else if scale >= 4. {
            4
        } else {
            1
        };
        Ok(Self {
            x,
            z,
            width,
            depth,
            span,
        })
    }
    pub fn contains(self, chunk: (i32, i32)) -> bool {
        let x = i64::from(chunk.0) * 16;
        let z = i64::from(chunk.1) * 16;
        x >= i64::from(self.x) - 16
            && x < i64::from(self.x) + i64::from(self.width) + 16
            && z >= i64::from(self.z) - 16
            && z < i64::from(self.z) + i64::from(self.depth) + 16
    }
    pub fn chunks(self, center: [f64; 3]) -> Vec<(i32, i32)> {
        let mut chunks = Vec::new();
        for z in (self.z - 16..self.z + i32::from(self.depth) + 16).step_by(16) {
            for x in (self.x - 16..self.x + i32::from(self.width) + 16).step_by(16) {
                if x >= -30_000_000
                    && z >= -30_000_000
                    && x + 15 <= 30_000_000
                    && z + 15 <= 30_000_000
                {
                    chunks.push((x.div_euclid(16), z.div_euclid(16)));
                }
            }
        }
        let at = (
            (center[0].floor() as i32).div_euclid(16),
            (center[2].floor() as i32).div_euclid(16),
        );
        chunks.sort_unstable_by_key(|&(x, z)| {
            (
                (x - at.0).abs().max((z - at.1).abs()),
                (x - at.0).abs() + (z - at.1).abs(),
            )
        });
        chunks
    }
}
pub(super) struct Image {
    pub grid: Grid,
    pub handle: iced::widget::image::Handle,
    pub atlas: Option<Arc<crate::textures::TextureAtlas>>,
}
pub(super) struct Output {
    pub tiles: Arc<Tiles>,
    pub image: Image,
}
pub(super) fn build(
    source: Arc<Session>,
    before: Arc<Tiles>,
    chunks: Vec<(i32, i32)>,
    grid: Grid,
    assets: Option<Arc<crate::textures::TextureBank>>,
    show_textures: bool,
    block_layers: crate::block_visibility::Layers,
    quality: crate::gpu_mesh::Quality,
    lighting: Arc<crate::lighting::Table>,
    cancelled: &AtomicBool,
) -> Result<Option<Output>, String> {
    let check = || cancelled.load(Ordering::Acquire);
    let mut tiles = before;
    if !chunks.is_empty() {
        let mut reader = source.source_reader().map_err(|e| e.to_string())?;
        for (cx, cz) in chunks {
            if check() {
                return Ok(None);
            }
            let patch = far_view::inspect_far_chunk_cached(
                &source,
                &mut reader,
                FarChunkRequest {
                    cx,
                    cz,
                    span: grid.span,
                },
                -2048,
                2047,
            )
            .map_err(|e| e.to_string())?;
            let mut surface = SurfaceView {
                biomes: Default::default(),
                x: cx * 16,
                z: cz * 16,
                width: 16,
                depth: 16,
                y_min: -2048,
                y_max: 2047,
                voxels: Vec::new(),
                missing_columns: 0,
                source_chunks: 0,
                source_tiles: Vec::new(),
            };
            far_view::append_chunk(&mut surface, patch);
            Arc::make_mut(&mut tiles).insert((cx, cz), Arc::new(surface));
        }
    }
    let bank = assets
        .as_ref()
        .filter(|_| show_textures)
        .map(|bank| bank.render_snapshot())
        .transpose()?;
    let atlas = bank.as_ref().and_then(|bank| bank.atlas()).cloned();
    let side = f32::from(grid.span);
    let size = Size::new(f32::from(grid.width) / side, f32::from(grid.depth) / side);
    let mut biomes = mcedit2_world::biome_source::Region::new(source.original_world().seed);
    if bank.is_some() {
        let geometry = source
            .original_world()
            .world_data
            .geometry(source.dimension())
            .map_err(|e| e.to_string())?;
        biomes.set_geometry(geometry);
        for surface in tiles.values() {
            for ((x, z), column) in surface.biomes.columns() {
                biomes.insert(x, z, Arc::clone(column));
            }
        }
    }
    let mut colors = bank.as_ref().map(|bank| bank.tint_context(&biomes));
    let mut vertices = Vec::new();
    for (ordinal, surface) in tiles.values().enumerate() {
        if ordinal % 16 == 0 && check() {
            return Ok(None);
        }
        if surface.x < grid.x
            || surface.z < grid.z
            || surface.x >= grid.x + i32::from(grid.width)
            || surface.z >= grid.z + i32::from(grid.depth)
        {
            continue;
        }
        for voxel in &surface.voxels {
            if !block_layers.visible(&voxel.state, assets.as_deref()) {
                continue;
            }
            let x = (i64::from(voxel.x) - i64::from(grid.x)) as f32 / side;
            let z = (i64::from(voxel.z) - i64::from(grid.z)) as f32 / side;
            let span = f32::from(voxel.span) / side;
            let style = crate::top_down::top_face_style(
                voxel,
                show_textures,
                bank.as_ref(),
                colors.as_mut(),
                atlas.as_deref(),
            );
            crate::projected_entities::grid_face(
                &mut vertices,
                [
                    Point::new(x, z),
                    Point::new(x + span, z),
                    Point::new(x + span, z + span),
                    Point::new(x, z + span),
                ],
                (2047. - voxel.y as f32) / 4096.,
                style.rgba,
                style.uv,
                style.mode,
                style.page,
                crate::lighting::FULLBRIGHT,
            );
        }
    }
    // 世界格网投影已经是像素坐标；光栅器的输入为标准化屏幕坐标。
    for vertex in &mut vertices {
        vertex.xy = [
            vertex.xy[0] / size.width * 2. - 1.,
            1. - vertex.xy[1] / size.height * 2.,
        ];
    }
    let Some(mut raster) = crate::canvas_raster::render_lit(
        &vertices,
        atlas.as_deref(),
        size,
        quality,
        &lighting,
        &check,
    )?
    else {
        return Ok(None);
    };
    for y in 0..raster.height {
        if y % 16 == 0 && check() {
            return Ok(None);
        }
        for x in 0..raster.width {
            let wx = grid.x + x as i32 * i32::from(grid.span);
            let wz = grid.z + y as i32 * i32::from(grid.span);
            let background = if !(-30_000_000..30_000_000).contains(&wx)
                || !(-30_000_000..30_000_000).contains(&wz)
            {
                [24u8, 24, 28]
            } else if let Some(tile) = tiles.get(&(wx.div_euclid(16), wz.div_euclid(16))) {
                let tile = &tile.source_tiles[0];
                if !tile.source_present {
                    [90, 40, 110]
                } else if tile.known_columns == 0 && tile.unknown_columns > 0 {
                    if (x + y) % 4 < 2 {
                        [145, 90, 25]
                    } else {
                        [65, 50, 30]
                    }
                } else {
                    [38, 42, 46]
                }
            } else if (x / 4 + y / 4) % 2 == 0 {
                [54, 58, 62]
            } else {
                [66, 70, 74]
            };
            let pixel = &mut raster.rgba[((y * raster.width + x) * 4) as usize..][..4];
            let alpha = u32::from(pixel[3]);
            for i in 0..3 {
                pixel[i] = (u32::from(pixel[i])
                    + (u32::from(background[i]) * (255 - alpha) + 127) / 255)
                    .min(255) as u8;
            }
            pixel[3] = 255;
        }
    }
    Ok(Some(Output {
        tiles,
        image: Image {
            grid,
            handle: raster.into_image(),
            atlas,
        },
    }))
}
