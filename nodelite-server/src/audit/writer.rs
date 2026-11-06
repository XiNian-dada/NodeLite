//! 审计日志后台写入任务。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use nodelite_proto::MAX_WRITER_BATCH_SIZE;
use rusqlite::{Connection, params};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tracing::{debug, warn};

use super::types::NewAuditEvent;

pub(super) struct AuditWriterContext {
    pub(super) connection: Arc<Mutex<Option<Connection>>>,
    pub(super) write_failures: Arc<AtomicU64>,
    pub(super) write_degraded: Arc<AtomicBool>,
    pub(super) batch_max: usize,
    pub(super) flush_interval: Duration,
}

pub(super) enum AuditWriterCommand {
    Event(NewAuditEvent),
    Flush(oneshot::Sender<()>),
}

pub(super) async fn run_audit_writer(
    mut rx: mpsc::Receiver<AuditWriterCommand>,
    context: AuditWriterContext,
) {
    let batch_max = context.batch_max.clamp(1, MAX_WRITER_BATCH_SIZE);
    let mut batch = Vec::with_capacity(batch_max);
    let mut flush_timer = tokio::time::interval(context.flush_interval);
    flush_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    flush_timer.tick().await;

    loop {
        tokio::select! {
            biased;
            received = rx.recv() => match received {
                Some(AuditWriterCommand::Event(event)) => {
                    batch.push(event);
                    if batch.len() >= batch_max {
                        flush_audit_batch(&mut batch, &context).await;
                    }
                }
                Some(AuditWriterCommand::Flush(ack)) => {
                    if !batch.is_empty() {
                        flush_audit_batch(&mut batch, &context).await;
                    }
                    let _ = ack.send(());
                }
                None => break,
            },
            _ = flush_timer.tick() => {
                if !batch.is_empty() {
                    flush_audit_batch(&mut batch, &context).await;
                }
            }
        }
    }

    while let Ok(command) = rx.try_recv() {
        match command {
            AuditWriterCommand::Event(event) => {
                batch.push(event);
                if batch.len() >= batch_max {
                    flush_audit_batch(&mut batch, &context).await;
                }
            }
            AuditWriterCommand::Flush(ack) => {
                if !batch.is_empty() {
                    flush_audit_batch(&mut batch, &context).await;
                }
                let _ = ack.send(());
            }
        }
    }
    if !batch.is_empty() {
        flush_audit_batch(&mut batch, &context).await;
    }
    debug!("audit writer task exited");
}

async fn flush_audit_batch(batch: &mut Vec<NewAuditEvent>, context: &AuditWriterContext) {
    if batch.is_empty() {
        return;
    }

    let events = std::mem::take(batch);
    let connection = Arc::clone(&context.connection);
    let result = tokio::task::spawn_blocking(move || {
        let mut guard = connection.blocking_lock();
        let Some(ref mut connection) = *guard else {
            anyhow::bail!("audit connection not initialized");
        };
        insert_event_batch(connection, &events)
    })
    .await;

    context
        .write_degraded
        .store(!matches!(&result, Ok(Ok(()))), Ordering::Release);
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            context.write_failures.fetch_add(1, Ordering::Relaxed);
            warn!(error = ?error, "failed to persist audit batch");
        }
        Err(error) => {
            context.write_failures.fetch_add(1, Ordering::Relaxed);
            warn!(error = ?error, "audit writer batch task join failed");
        }
    }
}

fn insert_event_batch(connection: &mut Connection, events: &[NewAuditEvent]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .context("failed to begin audit transaction")?;
    {
        let mut statement = transaction
            .prepare_cached(
                "INSERT INTO audit_log
                 (timestamp, event_type, user, node_id, ip_address, user_agent, success, details)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .context("failed to prepare audit insert")?;

        for event in events {
            let details = serde_json::to_string(&event.details)
                .context("failed to serialize audit details")?;
            statement
                .execute(params![
                    event.timestamp.timestamp(),
                    event.event_type.as_str(),
                    event.user,
                    event.node_id,
                    event.ip_address,
                    event.user_agent,
                    event.success as i64,
                    details,
                ])
                .context("failed to insert audit event")?;
        }
    }
    transaction
        .commit()
        .context("failed to commit audit transaction")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditEventType;

    #[tokio::test]
    async fn successful_batch_clears_current_failure_but_keeps_counter() {
        let connection = Connection::open_in_memory().expect("memory database");
        let context = AuditWriterContext {
            connection: Arc::new(Mutex::new(Some(connection))),
            write_failures: Arc::new(AtomicU64::new(0)),
            write_degraded: Arc::new(AtomicBool::new(false)),
            batch_max: 1,
            flush_interval: Duration::from_secs(1),
        };
        let event = || NewAuditEvent::now(AuditEventType::LoginFailure, "127.0.0.1", false);
        flush_audit_batch(&mut vec![event()], &context).await;
        assert!(context.write_degraded.load(Ordering::Acquire));
        context.connection.lock().await.as_ref().expect("connection").execute_batch(
            "CREATE TABLE audit_log (timestamp INTEGER, event_type TEXT, user TEXT, node_id TEXT, ip_address TEXT, user_agent TEXT, success INTEGER, details TEXT)"
        ).expect("repair schema");
        flush_audit_batch(&mut vec![event()], &context).await;
        assert!(!context.write_degraded.load(Ordering::Acquire));
        assert_eq!(context.write_failures.load(Ordering::Relaxed), 1);
    }
}
