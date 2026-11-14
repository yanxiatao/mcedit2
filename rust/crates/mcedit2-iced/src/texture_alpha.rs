//! 按不可变纹理源及采样合同缓存 Alpha 分类；弱引用不额外保活整包像素。
use super::*;
use std::sync::{RwLock, Weak};
#[derive(Debug)]
struct Snapshot {
    tiles: Weak<BTreeMap<String, Tile>>,
    simple: Weak<BTreeMap<String, Tile>>,
    animations: Weak<BTreeMap<String, SpriteAnimation>>,
    live: Weak<BTreeMap<String, Arc<texture_animation::Sprite>>>,
    modes: [std::collections::HashMap<String, (AlphaMode, AlphaMode)>; 2],
}
impl Snapshot {
    fn new(bank: &TextureBank) -> Self {
        Self {
            tiles: Arc::downgrade(&bank.tiles),
            simple: Arc::downgrade(&bank.simple_tiles),
            animations: Arc::downgrade(&bank.animations),
            live: Arc::downgrade(&bank.live_animations),
            modes: Default::default(),
        }
    }
    fn matches(&self, bank: &TextureBank) -> bool {
        self.tiles.as_ptr() == Arc::as_ptr(&bank.tiles)
            && self.simple.as_ptr() == Arc::as_ptr(&bank.simple_tiles)
            && self.animations.as_ptr() == Arc::as_ptr(&bank.animations)
            && self.live.as_ptr() == Arc::as_ptr(&bank.live_animations)
    }
}
#[derive(Debug, Default)]
pub(super) struct Cache(RwLock<Option<Snapshot>>);
impl Cache {
    pub(super) fn get(&self, bank: &TextureBank, id: &str) -> Option<(AlphaMode, AlphaMode)> {
        self.source(bank, id, false)
    }
    pub(super) fn simple(&self, bank: &TextureBank, id: &str) -> Option<AlphaMode> {
        self.source(bank, id, true).map(|value| value.0)
    }
    fn source(&self, bank: &TextureBank, id: &str, simple: bool) -> Option<(AlphaMode, AlphaMode)> {
        let domain = usize::from(simple);
        {
            let snapshot = self.0.read().expect("纹理 Alpha 分类缓存读锁异常");
            if let Some(found) = snapshot
                .as_ref()
                .filter(|value| value.matches(bank))
                .and_then(|value| value.modes[domain].get(id))
                .copied()
            {
                return Some(found);
            }
        }
        // 图片扫描不持有缓存锁，其他工作线程及点选不等待高分辨率图片。
        let mode = if simple {
            let pixels = bank.simple_image(id)?.alpha_mode();
            (pixels, pixels)
        } else {
            let pixels = bank.tiles.get(id)?.alpha_mode();
            let animation = bank
                .live_animations
                .get(id)
                .map(|sprite| sprite.alpha_mode())
                .or_else(|| bank.animations.get(id).map(SpriteAnimation::alpha_mode))
                .unwrap_or(pixels);
            (pixels, animation)
        };
        let mut snapshot = self.0.write().expect("纹理 Alpha 分类缓存写锁异常");
        let entry = snapshot.get_or_insert_with(|| Snapshot::new(bank));
        if !entry.matches(bank) {
            *entry = Snapshot::new(bank);
        }
        Some(*entry.modes[domain].entry(id.into()).or_insert(mode))
    }
}
