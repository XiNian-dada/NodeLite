//! 最后一次登录信息 API。

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Serialize;

use super::CurrentLoginEventId;
use crate::AppState;
use crate::audit::{AuditEvent, AuditEventType};
use crate::auth::{BASIC_AUTH_SESSION_COOKIE, TWO_FACTOR_AUTH_COOKIE, cookie_value};

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
    current_request_login: Option<Extension<CurrentLoginEventId>>,
) -> Result<Json<LastLoginInfo>, (StatusCode, String)> {
    // 从当前会话 cookie 获取登录事件 id,用于排除当前会话的登录。
    let current_login_event_id = current_request_login.map(|Extension(id)| id.0).or_else(|| {
        cookie_value(&headers, BASIC_AUTH_SESSION_COOKIE)
            .as_deref()
            .and_then(|token| {
                state
                    .two_factor_sessions
                    .get_basic_auth_login_event_id(token)
            })
            .or_else(|| {
                cookie_value(&headers, TWO_FACTOR_AUTH_COOKIE)
                    .as_deref()
                    .and_then(|token| {
                        state
                            .two_factor_sessions
                            .get_authenticated_login_event_id(token)
                    })
            })
    });

    // 查询所有 LoginSuccess 事件
    let query = crate::audit::AuditQuery {
        start: None,
        end: None,
        event_type: Some(AuditEventType::LoginSuccess),
        success: Some(true),
        limit: 100,
    };

    let events = state
        .audit_log
        .query(query)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let last_login_event = previous_login(&events, current_login_event_id);

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

fn previous_login(events: &[AuditEvent], current_id: Option<i64>) -> Option<&AuditEvent> {
    let current = current_id.and_then(|id| events.iter().find(|event| event.id == id));
    events.iter().find(|event| {
        let Some(current_id) = current_id else {
            return true;
        };
        if event.id >= current_id {
            return false;
        }
        let Some(current) = current else {
            return true;
        };
        // Basic Auth browsers can open several protected requests before the new
        // cookie arrives; those requests are one login burst, not prior logins.
        let same_basic_burst = current
            .details
            .get("basic_auth_only")
            .and_then(|v| v.as_bool())
            == Some(true)
            && event
                .details
                .get("basic_auth_only")
                .and_then(|v| v.as_bool())
                == Some(true)
            && event.ip_address == current.ip_address
            && event.user_agent == current.user_agent
            && (current.timestamp - event.timestamp).num_seconds().abs() <= 5;
        !same_basic_burst
    })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};
    use serde_json::json;

    use super::*;

    #[test]
    fn skips_parallel_basic_requests_from_the_same_browser_login() {
        let now = Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("timestamp");
        let login = |id, timestamp, agent: &str| AuditEvent {
            id,
            timestamp,
            event_type: AuditEventType::LoginSuccess,
            user: Some("viewer".to_string()),
            node_id: None,
            ip_address: "198.51.100.24".to_string(),
            user_agent: Some(agent.to_string()),
            success: true,
            details: json!({ "basic_auth_only": true }),
        };
        let events = [
            login(4, now + Duration::seconds(2), "browser"),
            login(3, now + Duration::seconds(1), "browser"),
            login(2, now, "browser"),
            login(1, now - Duration::minutes(1), "browser"),
        ];
        assert_eq!(
            previous_login(&events, Some(3)).map(|event| event.id),
            Some(1)
        );
    }
}
