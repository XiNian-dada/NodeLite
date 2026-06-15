//! 只读认证、2FA 校验与健康探针路由的窄模块拼装。

use axum::http::{HeaderMap, header};

mod healthz;
mod middleware;
mod two_factor;

pub(crate) use healthz::{healthz, readyz};
pub(crate) use middleware::require_readonly_auth;
pub(crate) use two_factor::{logout_and_reauth, verify_2fa_api};

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}
