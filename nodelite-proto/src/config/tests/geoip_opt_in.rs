//! GeoIP must stay off unless the operator opts in, because the online
//! provider sends public client IPs to a third party.

use super::super::{GeoIpProvider, ServerConfig, parse_server_config};

const BASE: &str =
    "[server]\nlisten = \"127.0.0.1:8080\"\npublic_base_url = \"http://127.0.0.1:8080\"\n";

fn parse(geoip: &str) -> ServerConfig {
    parse_server_config(&format!("{BASE}{geoip}")).expect("server config should parse")
}

#[test]
fn geoip_is_disabled_without_explicit_opt_in() {
    assert!(!parse("").geoip.enabled);
    assert!(!parse("[geoip]\nedition = \"country-lite\"\n").geoip.enabled);
}

#[test]
fn explicit_enabled_opts_into_online_provider() {
    let config = parse("[geoip]\nenabled = true\n");
    assert!(config.geoip.enabled);
    assert_eq!(config.geoip.provider, GeoIpProvider::Ipwhois);
}

#[test]
fn explicit_provider_without_enabled_keeps_legacy_configs_working() {
    let config = parse("[geoip]\nprovider = \"ipwhois\"\n");
    assert!(config.geoip.enabled);
    assert_eq!(config.geoip.provider, GeoIpProvider::Ipwhois);
}

#[test]
fn explicit_disable_wins_over_provider() {
    let config = parse("[geoip]\nenabled = false\nprovider = \"ipwhois\"\n");
    assert!(!config.geoip.enabled);
}
