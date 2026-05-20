use axum::extract::{Path as AxumPath, State};
use axum::http::header;
use axum::response::{AppendHeaders, Html, IntoResponse, Response};

use crate::AppState;
use crate::startup::PROTECTED_CACHE_CONTROL;
use crate::ui::{
    UI_I18N_JSON, index_html, index_page_csp, node_html, node_page_csp, verify_2fa_html,
    verify_2fa_page_csp,
};

const BRAND_LOGO_LIGHT_WEBP: &[u8] = include_bytes!("../../../logo/brand-logo-light.webp");
const BRAND_LOGO_DARK_WEBP: &[u8] = include_bytes!("../../../logo/brand-logo-dark.webp");

/// 首页 HTML:把刷新周期等参数注入模板。
pub(crate) async fn index(State(state): State<AppState>) -> Response {
    html_page_response(
        index_page_csp(),
        index_html(state.shared.config().refresh_interval_secs),
    )
}

/// 节点详情页 HTML。
pub(crate) async fn node_detail(
    State(state): State<AppState>,
    AxumPath(node_id): AxumPath<String>,
) -> Response {
    html_page_response(
        node_page_csp(),
        node_html(&node_id, state.shared.config().refresh_interval_secs),
    )
}

/// 把前端 i18n 字典作为静态 JSON 文件提供。
pub(crate) async fn ui_i18n_asset() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        UI_I18N_JSON,
    )
        .into_response()
}

pub(crate) async fn brand_logo_light_asset() -> Response {
    webp_asset(BRAND_LOGO_LIGHT_WEBP)
}

pub(crate) async fn brand_logo_dark_asset() -> Response {
    webp_asset(BRAND_LOGO_DARK_WEBP)
}

/// 2FA 验证页面。
pub(crate) async fn verify_2fa_page() -> Response {
    html_page_response(verify_2fa_page_csp(), verify_2fa_html())
}

fn webp_asset(bytes: &'static [u8]) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
        .into_response()
}

fn html_page_response<T>(csp: &'static str, body: T) -> Response
where
    Html<T>: IntoResponse,
{
    (
        AppendHeaders([
            (header::CONTENT_SECURITY_POLICY, csp),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "strict-origin-when-cross-origin"),
            (header::CACHE_CONTROL, PROTECTED_CACHE_CONTROL),
            (header::PRAGMA, "no-cache"),
        ]),
        Html(body),
    )
        .into_response()
}
