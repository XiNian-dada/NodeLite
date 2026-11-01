//! 安装/升级命令中发布源与固定校验和的渲染测试。

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;

use nodelite_proto::ServerConfig;

use crate::registry::{AgentReleaseSource, render_install_command, render_upgrade_command};
use crate::test_support::test_server_config;

const X86_SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const ARM_SHA: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

pub(super) fn release_config(mirror: Option<&str>) -> ServerConfig {
    let mut config = test_server_config(
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
        "https://monitor.example.com".to_string(),
        PathBuf::from("registry.json"),
        PathBuf::from("history.sqlite3"),
        PathBuf::from("snapshot.json"),
    );
    if let Some(mirror) = mirror {
        config.agent_release_base_url = Some(mirror.to_string());
        config.agent_release_sha256_x86_64 = Some(X86_SHA.to_string());
        config.agent_release_sha256_aarch64 = Some(ARM_SHA.to_string());
    }
    config
}

#[test]
fn default_release_source_keeps_github_latest_without_pins() {
    let release =
        AgentReleaseSource::from_config(&release_config(None)).expect("default source builds");
    let command = render_install_command("https://monitor.example.com", "tok", &release)
        .expect("install command renders");

    assert!(command.ends_with(
        "  --base-url 'https://github.com/XiNian-dada/NodeLite/releases/latest/download'"
    ));
    assert!(!command.contains("--sha256"));
}

#[test]
fn configured_mirror_is_used_with_pinned_checksums() {
    let release = AgentReleaseSource::from_config(&release_config(Some(
        "https://mirror.example.com/nodelite/v1.2.3/",
    )))
    .expect("mirror source builds");
    let install = render_install_command("https://monitor.example.com", "tok", &release)
        .expect("install command renders");
    let upgrade =
        render_upgrade_command("https://monitor.example.com", &release).expect("upgrade renders");

    for command in [&install, &upgrade] {
        assert!(!command.contains("github.com"), "{command}");
        assert!(command.contains(&format!(
            "  --base-url 'https://mirror.example.com/nodelite/v1.2.3' \\\n  --sha256-x86_64 '{X86_SHA}' \\\n  --sha256-aarch64 '{ARM_SHA}'"
        )), "{command}");
        assert!(!command.ends_with('\\'), "{command}");
    }
}

#[test]
fn http_sources_carry_the_installer_insecure_opt_in() {
    let https = AgentReleaseSource::from_config(&release_config(None)).expect("default source");
    let install = render_install_command("https://monitor.example.com", "tok", &https)
        .expect("install command renders");
    assert!(!install.contains("--insecure-allow-http"), "{install}");

    let upgrade =
        render_upgrade_command("http://monitor.example.com", &https).expect("upgrade renders");
    assert!(upgrade.contains("  --insecure-allow-http"), "{upgrade}");
    assert!(!upgrade.ends_with('\\'), "{upgrade}");

    let http_mirror = AgentReleaseSource::from_config(&release_config(Some(
        "http://mirror.example.com/nodelite",
    )))
    .expect("mirror source builds");
    let install = render_install_command("https://monitor.example.com", "tok", &http_mirror)
        .expect("install command renders");
    assert!(install.contains("  --insecure-allow-http \\"), "{install}");
}

#[test]
fn mirror_without_checksums_is_refused() {
    let mut config = release_config(Some("https://mirror.example.com/nodelite"));
    config.agent_release_sha256_aarch64 = None;

    assert!(AgentReleaseSource::from_config(&config).is_err());
}
