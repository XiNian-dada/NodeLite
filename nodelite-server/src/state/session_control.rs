//! 在线会话控制命令与错误边界。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};

/// 每个 WebSocket 会话最多积压的控制命令数。
pub(crate) const SESSION_CONTROL_CHANNEL_CAPACITY: usize = 8;

/// 运行中的 WebSocket 会话可接收的控制命令。
pub(crate) enum SessionCommand {
    RefreshToken {
        response: oneshot::Sender<Result<SessionRefreshReply, String>>,
        refresh_permit: SessionRefreshPermit,
    },
}

/// 存在于 registry 中的在线会话控制入口。
#[derive(Clone, Debug)]
pub(crate) struct SessionControlHandle {
    tx: mpsc::Sender<SessionCommand>,
    refresh_pending: Arc<AtomicBool>,
}

impl SessionControlHandle {
    pub(crate) fn channel() -> (Self, mpsc::Receiver<SessionCommand>) {
        let (tx, rx) = mpsc::channel(SESSION_CONTROL_CHANNEL_CAPACITY);
        (Self::new(tx), rx)
    }

    pub(crate) fn new(tx: mpsc::Sender<SessionCommand>) -> Self {
        Self {
            tx,
            refresh_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn try_enqueue_refresh(
        &self,
        response: oneshot::Sender<Result<SessionRefreshReply, String>>,
    ) -> Result<(), SessionCommandError> {
        let refresh_permit = SessionRefreshPermit::acquire(Arc::clone(&self.refresh_pending))?;
        self.tx
            .try_send(SessionCommand::RefreshToken {
                response,
                refresh_permit,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SessionCommandError::QueueFull,
                mpsc::error::TrySendError::Closed(_) => SessionCommandError::SessionClosed,
            })
    }
}

pub(crate) struct SessionRefreshPermit {
    refresh_pending: Arc<AtomicBool>,
}

impl SessionRefreshPermit {
    fn acquire(refresh_pending: Arc<AtomicBool>) -> Result<Self, SessionCommandError> {
        refresh_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SessionCommandError::CommandInFlight)?;
        Ok(Self { refresh_pending })
    }
}

impl Drop for SessionRefreshPermit {
    fn drop(&mut self) {
        self.refresh_pending.store(false, Ordering::Release);
    }
}

/// 一次在线 token 续期完成后返回给调用方的摘要。
#[derive(Debug, Clone)]
pub(crate) struct SessionRefreshReply {
    pub token_expires_at: DateTime<Utc>,
}

/// 向在线节点下发控制命令时可能遇到的失败类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionCommandError {
    NodeOffline,
    SessionClosed,
    QueueFull,
    CommandInFlight,
}

impl std::fmt::Display for SessionCommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeOffline => f.write_str("node is offline"),
            Self::SessionClosed => f.write_str("node session is no longer available"),
            Self::QueueFull => f.write_str("node session control queue is full"),
            Self::CommandInFlight => f.write_str("node token refresh is already pending"),
        }
    }
}

impl std::error::Error for SessionCommandError {}

#[cfg(test)]
mod tests {
    use tokio::sync::{mpsc, oneshot};

    use super::{
        SESSION_CONTROL_CHANNEL_CAPACITY, SessionCommand, SessionCommandError, SessionControlHandle,
    };

    #[test]
    fn refresh_command_is_rejected_while_one_is_pending() {
        let (control, _rx) = SessionControlHandle::channel();
        let (first_tx, _first_rx) = oneshot::channel();

        control
            .try_enqueue_refresh(first_tx)
            .expect("first refresh should enqueue");

        let (second_tx, _second_rx) = oneshot::channel();
        let error = control
            .try_enqueue_refresh(second_tx)
            .expect_err("duplicate refresh should be rejected");
        assert_eq!(error, SessionCommandError::CommandInFlight);
    }

    #[tokio::test]
    async fn bounded_queue_reports_full_and_releases_refresh_permit() {
        let (tx, mut rx) = mpsc::channel(1);
        let control = SessionControlHandle::new(tx.clone());
        fill_control_queue(&tx).await;

        let (full_tx, _full_rx) = oneshot::channel();
        let error = control
            .try_enqueue_refresh(full_tx)
            .expect_err("full control queue should be rejected");
        assert_eq!(error, SessionCommandError::QueueFull);

        let Some(SessionCommand::RefreshToken {
            refresh_permit: _refresh_permit,
            ..
        }) = rx.recv().await
        else {
            panic!("prefilled refresh command should still be queued");
        };
        drop(_refresh_permit);

        let (retry_tx, _retry_rx) = oneshot::channel();
        control
            .try_enqueue_refresh(retry_tx)
            .expect("failed queue-full attempt should not leave refresh pending");
    }

    #[test]
    fn control_channel_has_fixed_capacity() {
        assert_eq!(SESSION_CONTROL_CHANNEL_CAPACITY, 8);
    }

    async fn fill_control_queue(tx: &mpsc::Sender<SessionCommand>) {
        let (response, _rx) = oneshot::channel();
        let prefill_control = SessionControlHandle::new(tx.clone());
        prefill_control
            .try_enqueue_refresh(response)
            .expect("prefill should enqueue one command");
    }
}
