//! 字符串池用于节省内存:将高重复度的字符串(如国家名、OS 名称、Agent 版本号)
//! 去重存储为 Arc<str>,避免每个节点独立分配。
//!
//! 典型场景:
//! - 1000 个节点都在同一个国家/城市 → 1000 份 String 克隆 → 1 份 Arc<str> + 1000 个 Arc 指针
//! - 所有节点使用同一 Agent 版本 → 内存占用从 ~20KB 降到 ~20 字节

use dashmap::DashMap;
use std::sync::Arc;

/// 字符串池:自动去重高重复度字符串。
///
/// 使用 DashMap 提供并发安全的无锁访问,适合 WebSocket 连接时并发注册节点。
#[derive(Debug, Default)]
pub struct StringPool {
    pool: DashMap<String, Arc<str>>,
}

impl StringPool {
    pub fn new() -> Self {
        Self {
            pool: DashMap::new(),
        }
    }

    /// 将字符串加入池中,返回 Arc<str>。
    /// 如果池中已存在相同字符串,返回已有的 Arc(引用计数 +1)。
    pub fn intern(&self, value: &str) -> Arc<str> {
        if value.is_empty() {
            return Arc::from("");
        }

        // 快速路径:如果已存在,直接返回
        if let Some(entry) = self.pool.get(value) {
            return Arc::clone(entry.value());
        }

        // 慢速路径:插入新条目
        let owned = value.to_string();
        let arc: Arc<str> = Arc::from(owned.as_str());

        // 使用 entry API 避免竞争:如果另一个线程同时插入了,使用它的
        self.pool
            .entry(owned)
            .or_insert_with(|| Arc::clone(&arc))
            .value()
            .clone()
    }

    /// 将 Option<String> intern 为 Option<Arc<str>>。
    #[allow(dead_code)]
    pub fn intern_option(&self, value: Option<&String>) -> Option<Arc<str>> {
        value.map(|s| self.intern(s))
    }

    /// 返回池中当前条目数(用于监控)。
    ///
    /// 注意:池会单调增长(只增不减),历史上出现过的字符串会一直保留到服务重启。
    /// 对于当前 intern 的低基数字段(GeoIP 国家/城市),这不是问题,但如果后续
    /// intern 高基数字段(如节点标签、主机名),需要考虑池大小监控或清理策略。
    pub fn len(&self) -> usize {
        self.pool.len()
    }

    /// 检查池是否为空(用于测试)。
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.pool.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_returns_static_arc() {
        let pool = StringPool::new();
        let result = pool.intern("");
        assert_eq!(result.as_ref(), "");
    }

    #[test]
    fn same_string_returns_same_arc() {
        let pool = StringPool::new();
        let first = pool.intern("China");
        let second = pool.intern("China");

        // Arc::ptr_eq 检查底层指针是否相同
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn different_strings_return_different_arcs() {
        let pool = StringPool::new();
        let china = pool.intern("China");
        let usa = pool.intern("United States");

        assert!(!Arc::ptr_eq(&china, &usa));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn concurrent_intern_deduplicates_correctly() {
        use std::sync::Arc as StdArc;
        use std::thread;

        let pool = StdArc::new(StringPool::new());
        let mut handles = Vec::new();

        // 10 个线程同时 intern 相同的字符串
        for _ in 0..10 {
            let pool_clone = StdArc::clone(&pool);
            handles.push(thread::spawn(move || pool_clone.intern("Linux")));
        }

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // 所有返回的 Arc 应该指向同一个底层字符串
        for i in 1..results.len() {
            assert!(Arc::ptr_eq(&results[0], &results[i]));
        }

        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn intern_option_handles_none() {
        let pool = StringPool::new();
        let result = pool.intern_option(None);
        assert!(result.is_none());
    }

    #[test]
    fn intern_option_handles_some() {
        let pool = StringPool::new();
        let value = "Beijing".to_string();
        let result = pool.intern_option(Some(&value));

        assert!(result.is_some());
        assert_eq!(result.unwrap().as_ref(), "Beijing");
        assert_eq!(pool.len(), 1);
    }
}
