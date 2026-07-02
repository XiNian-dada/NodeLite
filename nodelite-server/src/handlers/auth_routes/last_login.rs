//! 最后一次登录信息 API。

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;

use crate::AppState;
use crate::audit::AuditEventType;

#[derive(Debug, Clone, Serialize)]
pub struct LastLoginInfo {
    pub timestamp: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// 获取当前用户的最后一次登录信息(不包括本次登录)。
pub(crate) async fn last_login(
    State(state): State<AppState>,
) -> Result<Json<LastLoginInfo>, (StatusCode, String)> {
    // 查询最近 2 次 LoginSuccess 事件
    let query = crate::audit::AuditQuery {
        start: None,
        end: None,
        event_type: Some(AuditEventType::LoginSuccess),
        success: Some(true),
        limit: 2,
    };

    let events = state
        .audit_log
        .query(query)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 如果有至少 2 条记录,返回倒数第 2 条(最后一次登录,排除本次)
    // 如果只有 1 条,说明这是首次登录,返回空值
    let last_login_event = if events.len() >= 2 {
        Some(&events[events.len() - 2])
    } else {
        None
    };

    let info = if let Some(event) = last_login_event {
        let details = &event.details;
        LastLoginInfo {
            timestamp: Some(event.timestamp.to_rfc3339()),
            ip_address: Some(event.ip_address.clone()),
            user_agent: event.user_agent.clone(),
            country: details
                .get("country")
                .and_then(|v| v.as_str())
                .map(String::from),
            city: details
                .get("city")
                .and_then(|v| v.as_str())
                .map(String::from),
            latitude: details.get("latitude").and_then(|v| v.as_f64()),
            longitude: details.get("longitude").and_then(|v| v.as_f64()),
        }
    } else {
        LastLoginInfo {
            timestamp: None,
            ip_address: None,
            user_agent: None,
            country: None,
            city: None,
            latitude: None,
            longitude: None,
        }
    };

    Ok(Json(info))
}
