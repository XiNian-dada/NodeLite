//! 巡检峰值查询独立于图表均值缓存，复用查询并发限额。

use chrono::{DateTime, Utc};

use super::*;

impl HistoryStore {
    pub(crate) async fn query_history_peaks(
        &self,
        node_id: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> HistoryResult<Vec<HistoryPoint>> {
        if !self.is_available() {
            return Ok(Vec::new());
        }
        let start =
            start.max(Utc::now() - chrono::Duration::hours(DEFAULT_HISTORY_RETENTION_HOURS as i64));
        let end = end.min(Utc::now());
        if end <= start {
            return Ok(Vec::new());
        }
        let permit = self.acquire_query_permit().await?;
        let db_path = Arc::clone(&self.db_path);
        let node_id = node_id.to_owned();
        let timeout = self.sqlite_busy_timeout_secs;
        let cache_kib = self.history_read_cache_kib;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let connection = open_read_connection(db_path.as_ref(), timeout, cache_kib)
                .map_err(HistoryError::Query)?;
            query::query_history_peaks_between(&connection, &node_id, start, end)
                .map_err(HistoryError::from)
        })
        .await
        .map_err(|error| {
            HistoryError::TaskFailed(anyhow!("history peak query task failed: {error}"))
        })?
    }
}
