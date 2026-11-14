//! 拖放载荷绑定真实库存快照或当前页预设；重绘复用完整物品，不重复深拷贝 NBT。
use mcedit2_world::inventory::{Context, Snapshot, Stack};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

type Item = Result<Arc<Stack>, String>;
struct Slots {
    snapshot: Weak<Snapshot>,
    items: BTreeMap<i16, Item>,
}
struct Presets {
    context: Context,
    items: BTreeMap<usize, Item>,
}
#[derive(Default)]
pub(super) struct Cache {
    slots: Mutex<BTreeMap<String, Slots>>,
    presets: Mutex<Option<Presets>>,
}
impl Cache {
    pub(super) fn clear(&self) {
        self.slots.lock().expect("槽位拖放缓存锁被毒化").clear();
        *self.presets.lock().expect("预设拖放缓存锁被毒化") = None;
    }
    pub(super) fn slot(&self, snapshot: &Arc<Snapshot>, slot: i16) -> Item {
        let mut cache = self.slots.lock().expect("槽位拖放缓存锁被毒化");
        let saved = cache
            .entry(snapshot.spec.path.clone())
            .or_insert_with(|| Slots {
                snapshot: Arc::downgrade(snapshot),
                items: BTreeMap::new(),
            });
        if saved.snapshot.as_ptr() != Arc::as_ptr(snapshot) {
            *saved = Slots {
                snapshot: Arc::downgrade(snapshot),
                items: BTreeMap::new(),
            };
        }
        saved
            .items
            .entry(slot)
            .or_insert_with(|| {
                let result = snapshot.copy(slot).map(Arc::new).map_err(|e| e.to_string());
                if let Err(error) = &result {
                    log::error!("[inventory-drag] {error}");
                }
                result
            })
            .clone()
    }
    pub(super) fn presets(
        &self,
        context: &Context,
        indices: &[usize],
    ) -> Result<BTreeMap<usize, Arc<Stack>>, String> {
        let catalog =
            crate::item_presets::get(context.version).ok_or("该数据版本尚无完整物品预设")?;
        let mut cache = self.presets.lock().expect("预设拖放缓存锁被毒化");
        if cache.as_ref().is_none_or(|saved| &saved.context != context) {
            *cache = Some(Presets {
                context: context.clone(),
                items: BTreeMap::new(),
            });
        }
        let saved = cache.as_mut().expect("预设缓存已绑定实际来源");
        // 只保留当前可见页；进行中的手势自行持有其已冻结载荷。
        saved.items.retain(|index, _| indices.contains(index));
        indices
            .iter()
            .map(|&index| {
                let item = saved
                    .items
                    .entry(index)
                    .or_insert_with(|| {
                        let result = catalog
                            .presets
                            .get(index)
                            .ok_or("物品预设索引无效".to_owned())
                            .and_then(|preset| preset.stack(context.clone()))
                            .map(Arc::new);
                        if let Err(error) = &result {
                            log::error!("[inventory-drag] {error}");
                        }
                        result
                    })
                    .clone()?;
                Ok((index, item))
            })
            .collect()
    }
}
