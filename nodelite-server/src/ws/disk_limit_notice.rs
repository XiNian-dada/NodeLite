//! 容量裁剪的会话级提示。

use anyhow::anyhow;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::SplitSink;
use nodelite_proto::{NoticeLevel, ServerNoticeMessage, WireMessage};

use super::ProtocolError;
use super::transport::send_message;

/// 裁剪在每个上报周期都会重复，每会话只提示一次，避免刷屏 Agent 日志。
pub(super) async fn notify_disks_over_limit_once(
    sender: &mut SplitSink<WebSocket, Message>,
    already_notified: &mut bool,
    dropped: u32,
    limit: usize,
) -> Result<(), ProtocolError> {
    if dropped == 0 || *already_notified {
        return Ok(());
    }
    *already_notified = true;
    let notice = WireMessage::ServerNotice(ServerNoticeMessage {
        level: NoticeLevel::Warn,
        code: None,
        message: format!("{dropped} disks over the limit of {limit} were dropped"),
    });
    let payload = serde_json::to_string(&notice)
        .map_err(|error| anyhow!("failed to serialize disk limit notice: {error}"))?;
    send_message(
        sender,
        Message::Text(payload.into()),
        "failed to send disk limit notice",
    )
    .await?;
    Ok(())
}
