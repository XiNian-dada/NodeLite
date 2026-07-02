//! 最后一次登录信息 API。

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::Serialize;

use crate::AppState;
use crate::audit::AuditEventType;
use crate::auth::{BASIC_AUTH_SESSION_COOKIE, cookie_value};

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
    headers: HeaderMap,
) -> Result<Json<LastLoginInfo>, (StatusCode, String)> {
    // 从当前会话 cookie 获取登录时间戳,用于排除当前会话的登录
    let current_login_timestamp = cookie_value(&headers, BASIC_AUTH_SESSION_COOKIE)
        .as_deref()
        .and_then(|token| {
            state
                .two_factor_sessions
                .get_basic_auth_login_timestamp(token)
        });

    // 查询所有 LoginSuccess 事件
    let query = crate::audit::AuditQuery {
        start: None,
        end: None,
        event_type: Some(AuditEventType::LoginSuccess),
        success: Some(true),
        limit: 10, // 多查一些以防当前登录在前几条
    };

    let events = state
        .audit_log
        .query(query)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 找到第一个不是当前会话的登录事件
    let last_login_event = events.iter().find(|event| {
        if let Some(current_ts) = current_login_timestamp {
            // 排除时间戳匹配的事件(允许1秒误差)
            let diff = (event.timestamp.timestamp() - current_ts.timestamp()).abs();
            diff > 1
        } else {
            // 没有当前会话(例如2FA模式),返回最近的一个
            true
        }
    });

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
