//! 外观结果的活动集与最近离开视野的结果；活动素材完整保留，预算只约束非活动缓存。
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
#[derive(Clone, Copy, serde::Serialize)]
pub(crate) struct Statistics {
    pub active: usize,
    pub recent: usize,
    pub recent_bytes: usize,
    pub restores: u64,
    pub evictions: u64,
}
struct Entry<K, V> {
    key: K,
    value: V,
    bytes: usize,
}
pub(crate) struct WorkingSet<K, V> {
    wanted: BTreeSet<K>,
    active: Arc<BTreeMap<K, V>>,
    recent: VecDeque<Entry<K, V>>,
    bytes: usize,
    budget: usize,
    restores: u64,
    evictions: u64,
}
impl<K: Ord + Clone, V: Clone> WorkingSet<K, V> {
    pub fn new(budget: usize) -> Self {
        Self {
            wanted: BTreeSet::new(),
            active: Arc::new(BTreeMap::new()),
            recent: VecDeque::new(),
            bytes: 0,
            budget,
            restores: 0,
            evictions: 0,
        }
    }
    pub fn active(&self) -> &Arc<BTreeMap<K, V>> {
        &self.active
    }
    pub fn contains(&self, key: &K) -> bool {
        self.active.contains_key(key)
    }
    fn remember(&mut self, key: K, value: V, bytes: usize) {
        if let Some(index) = self.recent.iter().position(|entry| entry.key == key) {
            self.bytes -= self.recent.remove(index).unwrap().bytes;
        }
        let overhead = std::mem::size_of::<Entry<K, V>>();
        if bytes > self.budget || overhead > self.budget - bytes {
            return;
        }
        let bytes = bytes + overhead;
        while self.bytes > self.budget - bytes {
            let old = self
                .recent
                .pop_front()
                .expect("非活动外观缓存计数必须对应条目");
            self.bytes -= old.bytes;
            self.evictions += 1;
        }
        self.bytes += bytes;
        self.recent.push_back(Entry { key, value, bytes });
    }
    /// 结果只发布到仍请求它的活动集；已离开视野的同代结果进入最近缓存。
    pub fn publish(&mut self, key: K, value: V, bytes: usize) -> bool {
        if self.wanted.contains(&key) {
            Arc::make_mut(&mut self.active).insert(key, value);
            true
        } else {
            self.remember(key, value, bytes);
            false
        }
    }
    pub fn select(&mut self, wanted: BTreeSet<K>, cost: impl Fn(&V) -> usize) -> bool {
        if wanted == self.wanted {
            return false;
        }
        let previous = Arc::clone(&self.active);
        for (key, value) in previous.iter().filter(|(key, _)| !wanted.contains(*key)) {
            self.remember(key.clone(), value.clone(), cost(value));
        }
        let mut active = BTreeMap::new();
        for key in &wanted {
            if let Some(value) = previous.get(key) {
                active.insert(key.clone(), value.clone());
            } else if let Some(index) = self.recent.iter().position(|entry| &entry.key == key) {
                let entry = self.recent.remove(index).unwrap();
                self.bytes -= entry.bytes;
                self.restores += 1;
                active.insert(entry.key, entry.value);
            }
        }
        self.wanted = wanted;
        self.active = Arc::new(active);
        true
    }
    pub fn statistics(&self) -> Statistics {
        Statistics {
            active: self.active.len(),
            recent: self.recent.len(),
            recent_bytes: self.bytes,
            restores: self.restores,
            evictions: self.evictions,
        }
    }
    pub fn clear(&mut self) {
        self.wanted.clear();
        self.active = Arc::new(BTreeMap::new());
        self.recent = VecDeque::new();
        self.bytes = 0;
        self.restores = 0;
        self.evictions = 0;
    }
}
