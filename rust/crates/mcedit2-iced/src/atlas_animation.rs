//! 不改变 UV 或世界几何的动画快照；只生成变化素材的独立小页和 mip。
use super::{
    runtime_atlas,
    texture_animation::{Sprite, Stamp},
    AtlasUv, RuntimeTexture, TextureAtlas,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(crate) struct Binding {
    pub sprite: Arc<Sprite>,
    pub uv: AtlasUv,
    frames: BTreeMap<u32, runtime_atlas::Page>,
}
pub(crate) type SampleStamp = Stamp;
#[derive(Debug)]
pub(crate) struct Sample {
    pub stamp: SampleStamp,
    pub page: runtime_atlas::Page,
}
#[derive(Debug)]
pub(crate) struct Frame {
    pub tick: u64,
    pub samples: BTreeMap<(u32, usize), Arc<Sample>>,
}
#[derive(Debug)]
pub(crate) struct Set {
    start: Instant,
    pub bindings: BTreeMap<(u32, usize), Arc<Binding>>,
    latest: Mutex<Arc<Frame>>,
}
impl Set {
    pub fn install(
        atlas: &mut TextureAtlas,
        sources: &BTreeMap<String, Arc<Sprite>>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), String> {
        let mut bindings = BTreeMap::new();
        let mut samples = BTreeMap::new();
        let mut reserve = 0_u64;
        for (id, sprite) in sources {
            if cancelled() {
                return Err("动画图集建立已取消".into());
            }
            let Some(uv) = atlas.entries.get(id) else {
                continue;
            };
            let mip = atlas.mip_page(uv.page)?;
            let owner = mip
                .owner_at(uv.sample(0.5, 0.5))?
                .ok_or("动画材质没有 owner")?;
            let material = &mip.materials[owner];
            if material.base_uv != [uv.start[0], uv.start[1], uv.end[0], uv.end[1]] {
                return Err(format!("动画材质 {id} 与实际图集槽不一致"));
            }
            let width = ((uv.end[0] - uv.start[0]) * mip.base.width as f32 + 1.).round() as u32;
            let height = ((uv.end[1] - uv.start[1]) * mip.base.height as f32 + 1.).round() as u32;
            let stamp = sprite.sequence.stamp(0);
            let mut frames = BTreeMap::new();
            let mut workspace = 0;
            // 每个实际被使用的源帧独立生成 mip 一次；正常播放只换 Arc，不在 20Hz 时钟里重建 mip。
            for index in sprite.indices() {
                if cancelled() {
                    return Err("动画源帧准备已取消".into());
                }
                let frame_bytes = u64::from(width) * u64::from(height) * 4;
                if atlas
                    .budget_bytes
                    .saturating_sub(atlas.allocated_bytes + reserve)
                    < frame_bytes
                {
                    return Err(format!("动画素材 {id} 的下一源帧超过资源预算"));
                }
                let tile = sprite.tile(Stamp {
                    index,
                    next: index,
                    elapsed: 0,
                    duration: 1,
                });
                let pixels = super::padded_tile(&tile, width, height, true, &mut || {
                    if cancelled() {
                        Err("动画帧重采样已取消".into())
                    } else {
                        Ok(())
                    }
                })?;
                let page = runtime_atlas::build_page(
                    &RuntimeTexture {
                        sampler: None,
                        id: id.clone(),
                        width,
                        height,
                        rgba: pixels.into_iter().flatten().collect(),
                    },
                    atlas.allocated_bytes + reserve,
                    atlas.budget_bytes,
                    cancelled,
                )?;
                if workspace == 0 {
                    workspace = page.bytes.checked_mul(3).ok_or("动画插值工作集预算溢出")?;
                    reserve = reserve.checked_add(workspace).ok_or("动画页预算溢出")?;
                }
                reserve = reserve.checked_add(page.bytes).ok_or("动画源帧预算溢出")?;
                if atlas
                    .allocated_bytes
                    .checked_add(reserve)
                    .is_none_or(|n| n > atlas.budget_bytes)
                {
                    return Err(format!("动画素材 {id} 的源帧和插值工作集超过资源预算"));
                }
                frames.insert(index, page);
            }
            let page = frames[&stamp.index].clone();
            if atlas
                .allocated_bytes
                .checked_add(reserve)
                .is_none_or(|n| n > atlas.budget_bytes)
            {
                return Err("动画图集工作集超过资源预算".into());
            }
            let key = (uv.page, owner);
            if bindings
                .insert(
                    key,
                    Arc::new(Binding {
                        sprite: Arc::clone(sprite),
                        uv: uv.clone(),
                        frames,
                    }),
                )
                .is_some()
            {
                return Err(format!("动画素材 {id} 重复使用图集槽"));
            }
            samples.insert(key, Arc::new(Sample { stamp: stamp, page }));
        }
        if !bindings.is_empty() {
            atlas.allocated_bytes += reserve;
            atlas.animation = Some(Arc::new(Self {
                start: Instant::now(),
                bindings,
                latest: Mutex::new(Arc::new(Frame { tick: 0, samples })),
            }));
        }
        Ok(())
    }
    fn uv(&self, key: &(u32, usize)) -> Option<&AtlasUv> {
        self.bindings.get(key).map(|binding| &binding.uv)
    }
    pub fn tick(&self) -> u64 {
        (self.start.elapsed().as_millis() / 50) as u64
    }
    pub fn tick_at(&self, at: Instant) -> u64 {
        (at.saturating_duration_since(self.start).as_millis() / 50) as u64
    }
    pub fn deadline(&self) -> Instant {
        self.start + Duration::from_millis((self.tick() + 1) * 50)
    }
    pub fn frame(&self, tick: u64) -> Result<Arc<Frame>, String> {
        let mut latest = self.latest.lock().expect("动画帧缓存锁被毒化");
        if latest.tick == tick {
            return Ok(Arc::clone(&latest));
        }
        let frame = self.frame_for(tick, self.bindings.keys().copied(), Some(&latest))?;
        *latest = Arc::clone(&frame);
        Ok(frame)
    }
    /// GPU 只准备已提交网格引用的图集槽，Canvas 仍可请求完整快照。
    pub(crate) fn frame_for(
        &self,
        tick: u64,
        keys: impl Iterator<Item = (u32, usize)>,
        previous: Option<&Frame>,
    ) -> Result<Arc<Frame>, String> {
        let mut samples = BTreeMap::new();
        for key in keys {
            let binding = &self.bindings[&key];
            let stamp = binding.sprite.sequence.stamp(tick);
            let old = previous.and_then(|frame| frame.samples.get(&key));
            let sample = if let Some(old) = old.filter(|old| old.stamp == stamp) {
                Arc::clone(old)
            } else {
                let a = &binding.frames[&stamp.index];
                let page = if stamp.index == stamp.next {
                    a.clone()
                } else {
                    interpolate(a, &binding.frames[&stamp.next], stamp)?
                };
                Arc::new(Sample { stamp: stamp, page })
            };
            samples.insert(key, sample);
        }
        Ok(Arc::new(Frame { tick, samples }))
    }
}
fn interpolate(
    a: &runtime_atlas::Page,
    b: &runtime_atlas::Page,
    stamp: Stamp,
) -> Result<runtime_atlas::Page, String> {
    let weight = 1. - f64::from(stamp.elapsed) / f64::from(stamp.duration);
    let mix = |a: &[u8], b: &[u8]| -> Result<Vec<u8>, String> {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(a.len())
            .map_err(|error| format!("动画插值内存分配失败：{error}"))?;
        for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            pixels.extend_from_slice(&[
                (weight * f64::from(a[0]) + (1. - weight) * f64::from(b[0])) as u8,
                (weight * f64::from(a[1]) + (1. - weight) * f64::from(b[1])) as u8,
                (weight * f64::from(a[2]) + (1. - weight) * f64::from(b[2])) as u8,
                a[3],
            ]);
        }
        Ok(pixels)
    };
    let base = Arc::new(super::AtlasPage {
        width: a.mip.base.width,
        height: a.mip.base.height,
        rgba: mix(&a.mip.base.rgba, &b.mip.base.rgba)?,
    });
    let mip = crate::material_mips::MipPage {
        base,
        rgba: mix(&a.mip.rgba, &b.mip.rgba)?,
        ..a.mip.as_ref().clone()
    };
    Ok(runtime_atlas::Page {
        mip: Arc::new(mip),
        ..a.clone()
    })
}
impl TextureAtlas {
    pub fn animation_deadline(&self) -> Option<Instant> {
        self.animation.as_ref().map(|set| set.deadline())
    }
    pub(crate) fn presented_animation_deadline(&self) -> Option<Instant> {
        if self
            .animation_frame
            .as_ref()
            .is_some_and(|frame| frame.samples.is_empty())
        {
            None
        } else {
            self.animation_deadline()
        }
    }
    pub fn animation_tick(&self) -> Option<u64> {
        if self
            .animation_frame
            .as_ref()
            .is_some_and(|frame| frame.samples.is_empty())
        {
            return None;
        }
        self.animation.as_ref().map(|set| {
            self.animation_frame
                .as_ref()
                .map_or_else(|| set.tick(), |frame| frame.tick)
        })
    }
    pub fn animation_is_current(&self) -> bool {
        self.animation.as_ref().is_none_or(|set| {
            self.animation_frame
                .as_ref()
                .is_some_and(|frame| frame.tick == set.tick())
        })
    }
    pub fn snapshot(&self) -> Result<Self, String> {
        self.snapshot_at(self.animation_tick().unwrap_or(0))
    }
    pub fn snapshot_at(&self, tick: u64) -> Result<Self, String> {
        let mut result = self.clone();
        if let Some(set) = &self.animation {
            result.animation_frame = Some(set.frame(tick)?);
        }
        Ok(result)
    }
    pub fn animation_sample(
        &self,
        uv: [f32; 2],
        page: u32,
        lod: f32,
        linear: bool,
        mode: u32,
    ) -> Result<Option<[f32; 4]>, String> {
        let Some(set) = &self.animation else {
            return Ok(None);
        };
        let Some(owner) = self.mip_page(page)?.owner_at(uv)? else {
            return Ok(None);
        };
        let key = (page, owner);
        let Some(uv_rect) = set.uv(&key) else {
            return Ok(None);
        };
        let frame = match &self.animation_frame {
            Some(frame) => Arc::clone(frame),
            None => set.frame(set.tick())?,
        };
        let Some(sample) = frame.samples.get(&key) else {
            return Ok(None);
        };
        let local: [f32; 2] = std::array::from_fn(|axis| {
            let span = uv_rect.end[axis] - uv_rect.start[axis];
            if span.abs() <= f32::EPSILON {
                0.5
            } else {
                ((uv[axis] - uv_rect.start[axis]) / span).clamp(0., 1.)
            }
        });
        sample
            .page
            .mip
            .sample_lod(sample.page.uv.sample(local[0], local[1]), lod, linear, mode)
            .map(Some)
    }
}
