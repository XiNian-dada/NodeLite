//! 静态 Web 资源交付模块：编译期内嵌并托管 Vue SPA 及前端资产。
//!
//! 产物目录 `web/dist/` 通过 `include_dir!` 宏打入二进制文件。
//! 本模块负责为单页应用入口与静态资源提供正确的 Content-Type、CSP 以及缓存响应头。

use std::sync::OnceLock;

use axum::{
    body::Body,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use include_dir::{Dir, include_dir};
use sha2::{Digest, Sha256};
use tracing::error;

/// 编译期内嵌自 `web/dist/` 的前端静态资源目录
static WEB_ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/web/dist");

/// 单页应用的 Content Security Policy（CSP）安全策略
const SPA_CSP: &str = "default-src 'self'; \
    img-src 'self' data:; \
    connect-src 'self' https://raw.githubusercontent.com https://api.github.com; \
    font-src 'self'; \
    object-src 'none'; \
    media-src 'none'; \
    worker-src 'none'; \
    base-uri 'none'; \
    frame-ancestors 'none'; \
    form-action 'self'";

/// SPA 入口页面的缓存控制策略（禁止缓存，保证实时性）
const NO_CACHE: &str = "no-store, no-cache, must-revalidate";

/// 带哈希版本签名的静态资产缓存策略（永久强缓存）
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// 交付 SPA 入口 index.html（服务于 `/` 与 `/nodes/:id` 路由）。
///
/// index.html 包含两段微型内联启动脚本（主题防闪烁与 24h 认证时间戳检查），
/// 必须在主 JS Bundle 加载前执行。我们在 CSP 中对其进行 sha256 锁定，兼顾安全与加载体验。
pub fn spa_index() -> Response {
    serve_file("index.html", NO_CACHE, spa_index_csp())
}

/// 交付独立的 2FA 验证页（`verify-2fa.html`）。
///
/// 该页面独立于 Vue SPA，为内联脚本与样式的自包含页面，在 SPA 加载前阻断未授权访问。
/// 同样采用基于 sha256 的 CSP 策略锁定内联代码。
pub fn verify_2fa_page() -> Response {
    let file = match WEB_ASSETS.get_file("verify-2fa.html") {
        Some(f) => f,
        None => {
            return (StatusCode::NOT_FOUND, "Not Found").into_response();
        }
    };

    finish_asset_response(
        "verify-2fa page",
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, NO_CACHE)
            .header(header::PRAGMA, "no-cache")
            .header(header::CONTENT_SECURITY_POLICY, verify_2fa_csp())
            .body(Body::from(file.contents())),
    )
}

/// 交付 `/assets/*` 路径下的前端静态资源
pub fn static_asset(path: &str) -> Response {
    // 路由捕获 /assets/ 后的所有内容，需补齐 "assets/" 前缀
    let full_path = format!("assets/{}", path);

    // 根据文件名特征决定缓存策略（带哈希内容采用强缓存，否则协商缓存）
    let cache_control = if is_hashed_asset(&full_path) {
        IMMUTABLE
    } else {
        NO_CACHE
    };

    serve_file(&full_path, cache_control, SPA_CSP)
}

/// 从内嵌资源目录中读取并组装 HTTP 响应
fn serve_file(path: &str, cache_control: &str, csp: &str) -> Response {
    let file = match WEB_ASSETS.get_file(path) {
        Some(f) => f,
        None => {
            return (StatusCode::NOT_FOUND, "Not Found").into_response();
        }
    };

    let content_type = mime_type_for_path(path);
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, cache_control)
        .header(header::CONTENT_SECURITY_POLICY, csp);
    // 兼容可能遵守 Pragma 的旧版 HTTP/1.0 代理，配合 no-cache 一同输出
    if cache_control == NO_CACHE {
        builder = builder.header(header::PRAGMA, "no-cache");
    }
    finish_asset_response(path, builder.body(Body::from(file.contents())))
}

fn finish_asset_response(asset: &str, response: Result<Response, axum::http::Error>) -> Response {
    match response {
        Ok(response) => response,
        Err(error) => {
            error!(error = ?error, asset, "failed to build web asset response");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

/// 判断路径是否为带有内容哈希签名的静态资产（可安全开启永久强缓存）。
///
/// Vite 构建产物规范为 `assets/<name>.<hash>.<ext>`，其中 `<hash>` 采用 base64url 字符集。
/// 仅当匹配包含 8 位以上哈希段的文件时赋予 immutable 缓存；未带哈希的入口文件（如 ui-i18n.json）则每次重新校验。
fn is_hashed_asset(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let segments: Vec<&str> = filename.split('.').collect();
    // 至少需要 `name` + `hash` + `ext` 3 部分
    if segments.len() < 3 {
        return false;
    }
    let hash = segments[segments.len() - 2];
    hash.len() >= 8
        && hash
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 根据文件扩展名返回对应的标准 MIME 类型
fn mime_type_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// 独立 2FA 页面的公共尾部 CSP 指令
const PAGE_CSP_DIRECTIVES: &str = "default-src 'self'; img-src 'self' data:; \
    connect-src 'self' https://raw.githubusercontent.com https://api.github.com; \
    font-src 'self'; object-src 'none'; media-src 'none'; worker-src 'none'; \
    base-uri 'none'; frame-ancestors 'none'; form-action 'self'";

static VERIFY_2FA_CSP: OnceLock<String> = OnceLock::new();

/// 计算（并缓存）内嵌 `verify-2fa.html` 的专属 CSP，对内联块哈希以确保与内容保持严格一致。
fn verify_2fa_csp() -> &'static str {
    VERIFY_2FA_CSP
        .get_or_init(|| {
            let template = WEB_ASSETS
                .get_file("verify-2fa.html")
                .map(|file| String::from_utf8_lossy(file.contents()).into_owned())
                .unwrap_or_default();
            build_page_csp(&template)
        })
        .as_str()
}

static SPA_INDEX_CSP: OnceLock<String> = OnceLock::new();

/// 计算（并缓存）SPA 外壳的 CSP：在 `SPA_CSP` 基础上增加显式的 `script-src 'self'`，
/// 并通过 sha256 锁定 index.html 的内联引导脚本（防闪烁 + 24h 认证检测）。
/// 仅放宽 script-src，style-src 严格保持 default-src 'self'。
fn spa_index_csp() -> &'static str {
    SPA_INDEX_CSP
        .get_or_init(|| {
            let html = WEB_ASSETS
                .get_file("index.html")
                .map(|file| String::from_utf8_lossy(file.contents()).into_owned())
                .unwrap_or_default();
            let script_hashes = extract_inline_tag_bodies(&html, "script")
                .into_iter()
                // 跳过外部 `<script src=…>`（空 body）；仅哈希真正的内联脚本
                .filter(|body| !body.trim().is_empty())
                .map(csp_hash)
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "default-src 'self'; script-src 'self' {script_hashes}; {}",
                SPA_CSP.trim_start_matches("default-src 'self'; ")
            )
        })
        .as_str()
}

/// 通过 sha256 锁定页面的内联 `<script>`/`<style>` 块，使严格的 CSP 策略依然允许其执行。
fn build_page_csp(template: &str) -> String {
    let script_hashes = extract_inline_tag_bodies(template, "script")
        .into_iter()
        .map(csp_hash)
        .collect::<Vec<_>>()
        .join(" ");
    let style_hashes = extract_inline_tag_bodies(template, "style")
        .into_iter()
        .map(csp_hash)
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "default-src 'self'; script-src 'self' {script_hashes}; \
         style-src 'self' 'unsafe-inline' {style_hashes}; \
         style-src-elem 'self' {style_hashes}; style-src-attr 'unsafe-inline'; {}",
        PAGE_CSP_DIRECTIVES.trim_start_matches("default-src 'self'; ")
    )
}

/// 提取所有 `<tag>…</tag>` 块的文本主体，精确匹配浏览器为 CSP `'sha256-…'` 源所计算的内容。
fn extract_inline_tag_bodies<'a>(document: &'a str, tag_name: &str) -> Vec<&'a str> {
    let mut bodies = Vec::new();
    let mut rest = document;
    let open_tag_prefix = format!("<{tag_name}");
    let close_tag = format!("</{tag_name}>");

    while let Some(open_index) = rest.find(&open_tag_prefix) {
        let after_open_start = &rest[open_index + open_tag_prefix.len()..];
        let Some(open_end_index) = after_open_start.find('>') else {
            break;
        };
        let after_open = &after_open_start[open_end_index + 1..];
        let Some(close_index) = after_open.find(&close_tag) else {
            break;
        };
        bodies.push(&after_open[..close_index]);
        rest = &after_open[close_index + close_tag.len()..];
    }

    bodies
}

/// Renders a single inline block as a CSP `'sha256-<base64>'` source token.
fn csp_hash(block: &str) -> String {
    let digest = Sha256::digest(block.as_bytes());
    format!("'sha256-{}'", STANDARD.encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_assets_exclude_source_maps() {
        let mut directories = vec![&WEB_ASSETS];
        while let Some(directory) = directories.pop() {
            for file in directory.files() {
                assert_ne!(
                    file.path().extension().and_then(|value| value.to_str()),
                    Some("map"),
                    "source map was embedded: {}",
                    file.path().display(),
                );
            }
            directories.extend(directory.dirs());
        }
    }

    #[test]
    fn test_is_hashed_asset() {
        // Vite 构建产物采用带 base64url 哈希的 <name>.<hash>.<ext> 格式。
        assert!(is_hashed_asset("assets/index.B_MrJhzj.js"));
        assert!(is_hashed_asset("assets/AccountView.B5MJM2zL.css"));
        assert!(is_hashed_asset("assets/index.CHYP72L6.css"));

        // 未带哈希的文件必须每次协商缓存，不能使用 immutable 永久强缓存。
        assert!(!is_hashed_asset("index.html"));
        assert!(!is_hashed_asset("verify-2fa.html"));
        assert!(!is_hashed_asset("assets/brand-logo-dark.webp"));
        assert!(!is_hashed_asset("assets/ui-i18n.json"));
    }

    #[test]
    fn test_mime_type_for_path() {
        assert_eq!(mime_type_for_path("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            mime_type_for_path("app.js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(mime_type_for_path("style.css"), "text/css; charset=utf-8");
        assert_eq!(
            mime_type_for_path("data.json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(mime_type_for_path("font.woff2"), "font/woff2");
        assert_eq!(mime_type_for_path("image.webp"), "image/webp");
    }

    #[test]
    fn test_spa_index_exists() {
        // 若 web/dist/index.html 不存在，会在编译期触发 include_str! 失败。
        let response = spa_index();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn test_spa_index_csp_pins_shim_without_relaxing_style() {
        let response = spa_index();
        let csp = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .expect("spa index should set a CSP")
            .to_str()
            .expect("CSP should be valid ascii");
        // index.html 中的内联 bootstrap shim 必须通过 sha256 锁定，避免在 script-src 'self' 下被拦截。
        assert!(csp.contains("script-src 'self' 'sha256-"), "csp={csp}");
        // 单页应用没有内联样式，style-src 必须保持严格，不可混入 'unsafe-inline'。
        assert!(!csp.contains("'unsafe-inline'"), "csp={csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "csp={csp}");
    }

    #[test]
    fn test_verify_2fa_page_pins_inline_blocks() {
        let response = verify_2fa_page();
        assert_eq!(response.status(), StatusCode::OK);
        let csp = response
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .expect("verify-2fa page should set a CSP")
            .to_str()
            .expect("CSP should be valid ascii");
        // 该页面包含内联 <script>/<style>，因此 CSP 必须对其分别进行定向放行或 sha256 锁定。
        assert!(csp.contains("script-src 'self' 'sha256-"), "csp={csp}");
        assert!(
            csp.contains("style-src 'self' 'unsafe-inline'"),
            "csp={csp}"
        );
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "csp={csp}"
        );
    }

    #[test]
    fn finish_asset_response_returns_500_on_builder_error() {
        let response = finish_asset_response(
            "broken asset",
            Response::builder()
                .header(header::CONTENT_TYPE, "bad\nvalue")
                .body(Body::empty()),
        );

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn build_page_csp_pins_scripts_and_allows_inline_styles() {
        let csp =
            build_page_csp("<html><style>body{color:red}</style><script>boot()</script></html>");
        assert!(csp.contains("script-src 'self' 'sha256-"), "csp={csp}");
        assert!(
            !csp.contains("script-src 'self' 'unsafe-inline'"),
            "csp={csp}"
        );
        assert!(
            csp.contains("style-src 'self' 'unsafe-inline' 'sha256-"),
            "csp={csp}"
        );
        assert!(csp.contains("style-src-attr 'unsafe-inline'"), "csp={csp}");
        assert!(
            csp.contains(
                "connect-src 'self' https://raw.githubusercontent.com https://api.github.com"
            ),
            "csp={csp}"
        );
    }

    #[test]
    fn extract_inline_tag_bodies_reads_each_block() {
        let bodies = extract_inline_tag_bodies(
            "<script type=\"module\">first()</script><script>second()</script>",
            "script",
        );
        assert_eq!(bodies, vec!["first()", "second()"]);
    }
}
