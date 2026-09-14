//! GeoIP provider and database behavior checked against local fixtures.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nodelite_proto::{GeoIpConfig, GeoIpEdition, GeoIpProvider};

use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::database::{
    database_type_matches_edition, dbip_download_url, dbip_download_urls, temporary_database_path,
};
use super::{GeoIpResolver, IpwhoisResponse, ipwhois_location_from_response, is_lan_ip};

#[test]
fn lan_ip_detection_covers_private_and_documentation_ranges() {
    for value in [
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.10",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.1.1",
        "192.0.2.1",
        "198.51.100.10",
        "203.0.113.20",
        "::1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
    ] {
        let ip: IpAddr = value.parse().expect("test ip should parse");
        assert!(is_lan_ip(ip), "{value} should be treated as LAN");
    }

    let public: IpAddr = "8.8.8.8".parse().expect("public ip");
    assert!(!is_lan_ip(public));
}

#[tokio::test]
async fn ipwhois_lookup_uses_online_api_and_cache() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let endpoint = format!(
        "http://{}",
        listener
            .local_addr()
            .expect("listener address should exist")
    );
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = Arc::clone(&hits);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("request should arrive");
        server_hits.fetch_add(1, Ordering::SeqCst);
        let mut request = [0_u8; 1024];
        let _ = stream
            .read(&mut request)
            .await
            .expect("request should read");
        let body = r#"{
            "success": true,
            "country": "United States",
            "country_code": "US",
            "region": "California",
            "city": "Mountain View",
            "latitude": 37.386,
            "longitude": -122.0838
        }"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("response should write");
    });
    let resolver = GeoIpResolver::new_with_ipwhois_endpoint(
        GeoIpConfig {
            enabled: true,
            provider: GeoIpProvider::Ipwhois,
            edition: GeoIpEdition::CountryLite,
            database_path: PathBuf::from("/definitely/missing/nodelite/ipwhois.mmdb"),
            auto_update: false,
            update_interval_days: 30,
        },
        endpoint,
    )
    .await;
    let ip: IpAddr = "8.8.8.8".parse().expect("test ip should parse");

    let first = resolver
        .lookup(ip)
        .await
        .expect("first lookup should resolve");
    let second = resolver
        .lookup(ip)
        .await
        .expect("second lookup should resolve");

    assert_eq!(first.country, "US");
    assert_eq!(first.city.as_deref(), Some("Mountain View"));
    assert_eq!(second, first);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    server.await.expect("server task should finish");
}

#[test]
fn ipwhois_response_maps_to_geoip_location() {
    let response: IpwhoisResponse = serde_json::from_str(
        r#"{
            "success": true,
            "country": "Hong Kong",
            "country_code": "HK",
            "region": "Central and Western",
            "city": "Hong Kong",
            "latitude": 22.3193,
            "longitude": 114.1694
        }"#,
    )
    .expect("ipwhois fixture should parse");

    let location = ipwhois_location_from_response(response).expect("location");

    assert_eq!(location.country, "HK");
    assert_eq!(location.city.as_deref(), Some("Hong Kong"));
    assert_eq!(location.latitude, Some(22.3193));
    assert_eq!(location.longitude, Some(114.1694));
}

#[test]
fn ipwhois_failed_response_is_ignored() {
    let response: IpwhoisResponse = serde_json::from_str(
        r#"{
            "success": false,
            "message": "Reserved range"
        }"#,
    )
    .expect("ipwhois fixture should parse");

    assert!(ipwhois_location_from_response(response).is_none());
}

#[test]
fn dbip_download_url_uses_requested_edition() {
    assert!(
        dbip_download_url(GeoIpEdition::CountryLite, 2026, 6)
            .contains("dbip-country-lite-2026-06.mmdb.gz")
    );
    assert!(
        dbip_download_url(GeoIpEdition::CityLite, 2026, 6)
            .contains("dbip-city-lite-2026-06.mmdb.gz")
    );
}

#[test]
fn dbip_database_type_must_match_requested_edition() {
    assert!(database_type_matches_edition(
        "DBIP-Country-Lite",
        GeoIpEdition::CountryLite
    ));
    assert!(database_type_matches_edition(
        "DBIP-City-Lite",
        GeoIpEdition::CityLite
    ));
    assert!(!database_type_matches_edition(
        "DBIP-Country-Lite",
        GeoIpEdition::CityLite
    ));
    assert!(!database_type_matches_edition(
        "DBIP-City-Lite",
        GeoIpEdition::CountryLite
    ));
}

#[test]
fn dbip_download_urls_roll_back_across_year_boundaries() {
    assert_eq!(
        dbip_download_urls(GeoIpEdition::CountryLite, 2026, 1, 3),
        vec![
            "https://download.db-ip.com/free/dbip-country-lite-2026-01.mmdb.gz".to_string(),
            "https://download.db-ip.com/free/dbip-country-lite-2025-12.mmdb.gz".to_string(),
            "https://download.db-ip.com/free/dbip-country-lite-2025-11.mmdb.gz".to_string(),
        ],
    );
}

#[test]
fn temporary_database_path_uses_sibling_tmp_file() {
    assert_eq!(
        temporary_database_path(Path::new("/var/lib/nodelite/geoip/dbip.mmdb")),
        Path::new("/var/lib/nodelite/geoip/dbip.mmdb.tmp"),
    );
}
