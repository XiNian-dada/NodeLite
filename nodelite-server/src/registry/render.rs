//! Agent 客户端配置与安装命令渲染模块。
//!
//! 根据服务端公网 URL 生成对应的 WebSocket 接入地址、渲染合法的 agent.toml 配置文本及安装 shell 命令。

use nodelite_proto::ServerConfig;
use url::Url;

use crate::encoding::shell_quote;

use super::{RegisteredNode, RegistryError, RegistryResult};

/// 从 `public_base_url` 推导 Agent 应连接的 WebSocket URL(http → ws,https → wss)。
pub fn build_agent_server_url(public_base_url: &str) -> RegistryResult<String> {
    let mut url = Url::parse(public_base_url)
        .map_err(|error| RegistryError::invalid_config("server.public_base_url", error))?;
    let scheme = match url.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => {
            return Err(RegistryError::UnsupportedPublicBaseUrlScheme(
                other.to_string(),
            ));
        }
    };
    url.set_scheme(scheme).map_err(|_| {
        RegistryError::internal(
            "failed to set websocket scheme",
            anyhow::anyhow!("url::Url refused to switch scheme"),
        )
    })?;
    url.set_path("/ws");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}

/// 拼装"安装脚本下载 URL"。
pub fn build_install_script_url(public_base_url: &str) -> RegistryResult<String> {
    let mut url = Url::parse(public_base_url)
        .map_err(|error| RegistryError::invalid_config("server.public_base_url", error))?;
    url.set_path("/install/install-agent.sh");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}

/// 拼装"安装引导 URL":Agent 安装脚本会带上 Bearer 安装令牌请求这个地址换取自己的 agent.toml。
pub fn build_install_bootstrap_url(public_base_url: &str) -> RegistryResult<String> {
    let mut url = Url::parse(public_base_url)
        .map_err(|error| RegistryError::invalid_config("server.public_base_url", error))?;
    url.set_path("/install/bootstrap");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}

/// 从 GitHub 仓库 URL 推导 `releases/latest/download` 形式的下载基地址。
/// 只支持 GitHub 仓库,避免误把任意 URL 当作发布源。
pub fn build_github_release_base_url(repository_url: &str) -> RegistryResult<String> {
    let url = Url::parse(repository_url)
        .map_err(|error| RegistryError::invalid_config("repository URL", error))?;
    let host = url.host_str().ok_or_else(|| {
        RegistryError::invalid_config("repository URL", "repository URL must include a host")
    })?;
    if host != "github.com" {
        return Err(RegistryError::invalid_config(
            "repository URL",
            "only GitHub repositories are supported for latest release installs",
        ));
    }

    let mut segments = url.path_segments().ok_or_else(|| {
        RegistryError::invalid_config(
            "repository URL",
            "repository URL must include an owner and repo",
        )
    })?;
    let owner = segments
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RegistryError::invalid_config("repository URL", "repository URL must include an owner")
        })?
        .to_string();
    let repo = segments
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RegistryError::invalid_config("repository URL", "repository URL must include a repo")
        })?
        .trim_end_matches(".git")
        .to_string();

    let mut release_url = url;
    release_url.set_path(&format!("{owner}/{repo}/releases/latest/download"));
    release_url.set_query(None);
    release_url.set_fragment(None);
    Ok(release_url.into())
}

/// 缺省下载源:由当前 crate 在编译期注入的仓库地址推导而来。
pub fn default_agent_release_base_url() -> RegistryResult<String> {
    build_github_release_base_url(env!("CARGO_PKG_REPOSITORY"))
}

/// 安装/升级命令使用的 Agent 发布源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReleaseSource {
    base_url: String,
    sha256_x86_64: Option<String>,
    sha256_aarch64: Option<String>,
}

impl AgentReleaseSource {
    /// 有 `[install]` 镜像时使用它并固定两个架构的 SHA256;否则回退到项目 GitHub latest,
    /// 由安装脚本读取发布源中的 SHA256SUMS.txt。
    pub fn from_config(config: &ServerConfig) -> RegistryResult<Self> {
        let Some(base_url) = config.agent_release_base_url.as_deref() else {
            return Ok(Self {
                base_url: default_agent_release_base_url()?,
                sha256_x86_64: None,
                sha256_aarch64: None,
            });
        };
        // Config validation already enforces this; re-checking keeps a mirror from
        // ever being rendered without pinned checksums.
        let (Some(sha256_x86_64), Some(sha256_aarch64)) = (
            config.agent_release_sha256_x86_64.clone(),
            config.agent_release_sha256_aarch64.clone(),
        ) else {
            return Err(RegistryError::invalid_config(
                "install.agent_release_base_url",
                "both agent release SHA256 values are required",
            ));
        };
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            sha256_x86_64: Some(sha256_x86_64),
            sha256_aarch64: Some(sha256_aarch64),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn push_args(&self, bootstrap_url: Option<&str>, lines: &mut Vec<String>) {
        let mut args = vec![format!("--base-url {}", shell_quote(&self.base_url))];
        // Config validation only admits remote http:// with server.insecure_allow_http;
        // carry that opt-in so the installer's HTTPS guard accepts the same sources.
        if bootstrap_url
            .into_iter()
            .chain([self.base_url.as_str()])
            .any(is_remote_http)
        {
            args.push("--insecure-allow-http".to_string());
        }
        if let Some(sha256) = self.sha256_x86_64.as_deref() {
            args.push(format!("--sha256-x86_64 {}", shell_quote(sha256)));
        }
        if let Some(sha256) = self.sha256_aarch64.as_deref() {
            args.push(format!("--sha256-aarch64 {}", shell_quote(sha256)));
        }
        let last = args.len() - 1;
        for (index, arg) in args.into_iter().enumerate() {
            let suffix = if index == last { "" } else { " \\" };
            lines.push(format!("  {arg}{suffix}"));
        }
    }
}

/// 安装脚本只对远程 `http://` 要求 `--insecure-allow-http`;loopback 与它的放行规则保持一致。
fn is_remote_http(url: &str) -> bool {
    let Ok(url) = Url::parse(url) else {
        return false;
    };
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    url.scheme() == "http" && !loopback
}

/// 渲染"复制即可用"的安装命令文本。
///
/// 输出形如多行 `curl ... | sh ...`,把安装令牌通过环境变量传递,避免它出现在 `ps` 列表中。
pub fn render_install_command(
    public_base_url: &str,
    install_token: &str,
    release: &AgentReleaseSource,
) -> RegistryResult<String> {
    let script_url = build_install_script_url(public_base_url)?;
    let bootstrap_url = build_install_bootstrap_url(public_base_url)?;
    let mut lines = vec![
        format!("curl -fsSL {} | \\", shell_quote(&script_url)),
        format!(
            "  NODELITE_AGENT_INSTALL_TOKEN={} sh -s -- \\",
            shell_quote(install_token)
        ),
        format!("  --bootstrap-url {} \\", shell_quote(&bootstrap_url)),
    ];
    release.push_args(Some(&bootstrap_url), &mut lines);

    Ok(lines.join("\n"))
}

/// 渲染就地升级 Agent 的命令文本。与 `render_install_command` 类似,但不需要令牌。
pub fn render_upgrade_command(
    public_base_url: &str,
    release: &AgentReleaseSource,
) -> RegistryResult<String> {
    let script_url = build_install_script_url(public_base_url)?;
    let mut lines = vec![
        format!("curl -fsSL {} | \\", shell_quote(&script_url)),
        "  NODELITE_AGENT_MODE=upgrade sh -s -- \\".to_string(),
        "  --mode upgrade \\".to_string(),
    ];
    release.push_args(None, &mut lines);

    Ok(lines.join("\n"))
}

/// 渲染单个节点的 `agent.toml` 文本,作为引导接口的响应体。
pub fn render_agent_config(
    public_base_url: &str,
    node: &RegisteredNode,
    plaintext_token: &str,
) -> RegistryResult<String> {
    let server_url = build_agent_server_url(public_base_url)?;
    let mut content = String::new();
    content.push_str("[agent]\n");
    content.push_str(&format!("node_id = \"{}\"\n", toml_escape(&node.node_id)));
    content.push_str(&format!(
        "node_label = \"{}\"\n",
        toml_escape(&node.node_label)
    ));
    content.push_str(&format!("server = \"{}\"\n", toml_escape(&server_url)));
    content.push_str(&format!("token = \"{}\"\n", toml_escape(plaintext_token)));
    content.push_str("report_interval_secs = 5\n");
    let filesystems = nodelite_proto::config::DEFAULT_AGENT_IGNORED_FILESYSTEMS
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(", ");
    content.push_str(&format!("ignored_filesystems = [{filesystems}]\n"));
    content.push_str(&format!(
        "auth_timeout_secs = {}\nsend_timeout_secs = {}\ninbound_timeout_secs = {}\n",
        nodelite_proto::config::DEFAULT_AGENT_AUTH_TIMEOUT_SECS,
        nodelite_proto::config::DEFAULT_AGENT_SEND_TIMEOUT_SECS,
        nodelite_proto::config::DEFAULT_AGENT_INBOUND_TIMEOUT_SECS,
    ));
    if !node.tags.is_empty() {
        let tags = node
            .tags
            .iter()
            .map(|tag| format!("\"{}\"", toml_escape(tag)))
            .collect::<Vec<_>>()
            .join(", ");
        content.push_str(&format!("tags = [{tags}]\n"));
    }
    Ok(content)
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
