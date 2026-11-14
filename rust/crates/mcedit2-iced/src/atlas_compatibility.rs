//! 所有视图、模型和导入预览共用不可变图集的前缀证明。
use super::TextureAtlas;
use std::sync::{Arc, Mutex, OnceLock, Weak};
type Pair = (Weak<TextureAtlas>, Weak<TextureAtlas>, bool);
fn lock(cache: &Mutex<Vec<Pair>>) -> std::sync::MutexGuard<'_, Vec<Pair>> {
    cache.lock().expect("图集前缀证明缓存锁异常")
}
/// 比较 UV、白色素材和全部旧页身份；昂贵检查不占用共享缓存锁。
/// 仅保存弱身份，既防止地址复用，也不保活任何纹理载荷。
pub(crate) fn atlas_compatible(extended: &Arc<TextureAtlas>, base: &Arc<TextureAtlas>) -> bool {
    if Arc::ptr_eq(extended, base) {
        return true;
    }
    static CACHE: OnceLock<Mutex<Vec<Pair>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let a = Arc::downgrade(extended);
    let b = Arc::downgrade(base);
    {
        let entries = lock(cache);
        if let Some((_, _, result)) = entries
            .iter()
            .find(|(x, y, _)| x.ptr_eq(&a) && y.ptr_eq(&b))
        {
            return *result;
        }
    }
    let result = extended.extends_exactly(base);
    let mut entries = lock(cache);
    entries.retain(|(x, y, _)| x.strong_count() > 0 && y.strong_count() > 0);
    if let Some((_, _, ready)) = entries
        .iter()
        .find(|(x, y, _)| x.ptr_eq(&a) && y.ptr_eq(&b))
    {
        return *ready;
    }
    entries.push((a, b, result));
    result
}
