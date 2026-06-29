use std::net::{IpAddr, Ipv6Addr};
use std::path::PathBuf;

use nodelite_proto::{GeoIpConfig, GeoIpEdition, GeoIpLocation, GeoIpProvider};

use super::{GeoIpResolver, IPWHOIS_CACHE_MAX_ENTRIES, IpwhoisClient};

#[tokio::test]
async fn ipwhois_provider_does_not_require_database_file() {
    let resolver = GeoIpResolver::new(GeoIpConfig {
        enabled: true,
        provider: GeoIpProvider::Ipwhois,
        edition: GeoIpEdition::CountryLite,
        database_path: PathBuf::from("/definitely/missing/nodelite/ipwhois.mmdb"),
        auto_update: false,
        update_interval_days: 30,
    })
    .await;

    assert!(resolver.prepare_database().await);
}

#[tokio::test]
async fn ipwhois_cache_entry_count_is_bounded() {
    let client = IpwhoisClient::new("http://127.0.0.1");
    let location = GeoIpLocation {
        country: "US".to_string(),
        city: Some("Mountain View".to_string()),
        latitude: Some(37.386),
        longitude: Some(-122.0838),
    };

    for index in 0..(IPWHOIS_CACHE_MAX_ENTRIES + 5) {
        client
            .cache_location(IpAddr::V6(Ipv6Addr::from(index as u128)), location.clone())
            .await;
    }

    assert_eq!(client.cache.read().await.len(), IPWHOIS_CACHE_MAX_ENTRIES);
}
