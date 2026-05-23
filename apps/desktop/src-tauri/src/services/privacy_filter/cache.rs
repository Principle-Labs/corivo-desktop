//! Classify-time span cache —— blake3 hash → Vec<PiiSpan> LRU。
//!
//! spec §9.1:AX 文本捕获重复率极高(用户停留同一页面 + typing
//! indicator 闪烁触发的重复 capture),不去重的话每次都重跑 195ms
//! 模型纯属浪费。期望 hit rate > 85%,把"naive 24 秒/小时 CPU"压到
//! "3-5 秒/小时"。
//!
//! 设计选择:
//! - blake3 而非 sha256:更快,32 字节 fingerprint 强度对缓存键足够。
//! - LRU 而非 LFU/2Q:实现简单,AX 文本天然 temporal locality 强(用户
//!   停留时间窗口),LRU 命中率已足够。
//! - 容量 10000 条 ≈ 5MB(spans JSON 平均几百字节)。spec 给的数字。
//! - 不持久化:进程重启清空。避免模型升级后旧 spans 跟新模型解码
//!   不一致 —— 重新计算成本远低于"用错误的缓存值发出去"。

use std::sync::Mutex;

use lru::LruCache;

use crate::domain::privacy::PiiSpan;

/// 缓存键 —— blake3 的 32 字节摘要。直接当 `[u8; 32]` 用,Hash + Eq
/// 都是 derive 默认实现,LRU 桶里跑得很快。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey([u8; 32]);

impl CacheKey {
    /// 对外只暴露 blake3 输入 —— 不允许直接构造,避免哈希算法被绕过。
    pub fn from_text(text: &str) -> Self {
        let hash = blake3::hash(text.as_bytes());
        Self(*hash.as_bytes())
    }
}

/// spec §9.1 容量。改这个值要同步更新 spec 里的数字。
const DEFAULT_CAPACITY: usize = 10_000;

pub struct SpanCache {
    inner: Mutex<LruCache<CacheKey, Vec<PiiSpan>>>,
}

impl SpanCache {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let cap = std::num::NonZeroUsize::new(capacity).expect("capacity > 0");
        Self {
            inner: Mutex::new(LruCache::new(cap)),
        }
    }

    /// 给输入文本算缓存键。封装这一步是为了让上层不直接依赖 blake3,
    /// 也方便将来切换算法。
    pub fn key_for(&self, text: &str) -> CacheKey {
        CacheKey::from_text(text)
    }

    /// 命中时返回 spans 的克隆(spans 不大,clone 比让外层 hold Mutex
    /// 锁好得多)。同时把这一项标记为最近使用 —— `LruCache::get` 会自
    /// 动 promote。
    pub fn get(&self, key: &CacheKey) -> Option<Vec<PiiSpan>> {
        self.inner.lock().unwrap().get(key).cloned()
    }

    pub fn put(&self, key: CacheKey, spans: Vec<PiiSpan>) {
        self.inner.lock().unwrap().put(key, spans);
    }

    /// 当前缓存条目数。监控用 —— spec §8 埋点要看 hit rate。
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 全清。Settings 页"清除隐私缓存"按钮的兜底路径,以及模型升级
    /// 后由 `download.rs` 调用(下一阶段)。
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

impl Default for SpanCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::privacy::PiiLabel;

    fn span_at(start: usize, end: usize) -> PiiSpan {
        PiiSpan {
            start,
            end,
            label: PiiLabel::PrivatePerson,
            score: 0.9,
            redacted_in_storage: false,
        }
    }

    #[test]
    fn same_text_yields_same_key() {
        let cache = SpanCache::new();
        let a = cache.key_for("hello");
        let b = cache.key_for("hello");
        assert_eq!(a, b);
    }

    #[test]
    fn different_text_yields_different_key() {
        let cache = SpanCache::new();
        let a = cache.key_for("hello");
        let b = cache.key_for("hello "); // 末尾多一个空格
        assert_ne!(a, b);
    }

    #[test]
    fn put_then_get_round_trips() {
        let cache = SpanCache::new();
        let key = cache.key_for("约一下 John");
        cache.put(key, vec![span_at(4, 8)]);
        let got = cache.get(&key).expect("cache miss after put");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].start, 4);
    }

    #[test]
    fn miss_returns_none_without_panic() {
        let cache = SpanCache::new();
        let key = cache.key_for("never inserted");
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn capacity_evicts_oldest() {
        let cache = SpanCache::with_capacity(2);
        let k1 = cache.key_for("a");
        let k2 = cache.key_for("b");
        let k3 = cache.key_for("c");
        cache.put(k1, vec![]);
        cache.put(k2, vec![]);
        cache.put(k3, vec![]); // 触发 k1 evict
        assert!(cache.get(&k1).is_none(), "k1 should be evicted");
        assert!(cache.get(&k2).is_some());
        assert!(cache.get(&k3).is_some());
    }

    #[test]
    fn clear_drops_all() {
        let cache = SpanCache::new();
        cache.put(cache.key_for("x"), vec![]);
        cache.put(cache.key_for("y"), vec![]);
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty());
    }
}
