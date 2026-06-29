//! GeoIP lookup, ipwhois online resolution, and DB-IP Lite database preparation.

use std::collections::HashMap;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Datelike;
use flate2::read::GzDecoder;
use maxminddb::geoip2;
use nodelite_proto::{GeoIpConfig, GeoIpEdition, GeoIpLocation, GeoIpProvider};
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::sanitize::sanitize_location_override;

const LAN_COUNTRY_CODE: &str = "LAN";
const DOWNLOAD_TIMEOUT_SECS: u64 = 30;
const DBIP_DOWNLOAD_ATTEMPTS: usize = 12;
const IPWHOIS_ENDPOINT: &str = "https://ipwho.is";
const IPWHOIS_TIMEOUT_SECS: u64 = 3;
const IPWHOIS_CACHE_TTL_SECS: u64 = 30 * 24 * 60 * 60;
const IPWHOIS_CACHE_MAX_ENTRIES: usize = 10_000;
const IPWHOIS_RETRY_AFTER_FALLBACK_SECS: u64 = 5 * 60;
const IPWHOIS_FIELDS: &str = "success,message,country,country_code,region,city,latitude,longitude";

type GeoIpReader = Arc<maxminddb::Reader<Vec<u8>>>;

#[derive(Clone)]
pub(crate) struct GeoIpResolver {
    config: GeoIpConfig,
    reader: Arc<RwLock<Option<GeoIpReader>>>,
    ipwhois: IpwhoisClient,
}

#[derive(Clone)]
struct IpwhoisClient {
    client: Option<reqwest::Client>,
    endpoint: Arc<str>,
    cache: Arc<RwLock<HashMap<IpAddr, CachedIpwhoisLocation>>>,
    retry_after: Arc<RwLock<Option<Instant>>>,
}

#[derive(Clone)]
struct CachedIpwhoisLocation {
    location: GeoIpLocation,
    expires_at: Instant,
}

#[derive(Debug, Deserialize)]
struct IpwhoisResponse {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
}

impl GeoIpResolver {
    pub(crate) async fn new(config: GeoIpConfig) -> Self {
        Self::new_with_ipwhois_endpoint(config, IPWHOIS_ENDPOINT).await
    }

    async fn new_with_ipwhois_endpoint(
        config: GeoIpConfig,
        ipwhois_endpoint: impl Into<Arc<str>>,
    ) -> Self {
        let resolver = Self {
            config,
            reader: Arc::new(RwLock::new(None)),
            ipwhois: IpwhoisClient::new(ipwhois_endpoint),
        };
        if resolver.uses_local_database() {
            resolver.reload_from_disk().await;
        }
        resolver
    }

    pub(crate) async fn prepare_database(&self) -> bool {
        if !self.config.enabled {
            return false;
        }
        if self.config.provider == GeoIpProvider::Ipwhois {
            return true;
        }
        if should_skip_download(&self.config) {
            return self.reload_from_disk().await;
        }
        match download_dbip_database(&self.config).await {
            Ok(()) => {
                info!(
                    path = %self.config.database_path.display(),
                    "geoip database downloaded"
                );
                self.reload_from_disk().await
            }
            Err(error) => {
                warn!(error = ?error, "failed to update geoip database; continuing without blocking startup");
                false
            }
        }
    }

    pub(crate) async fn lookup(&self, ip: IpAddr) -> Option<GeoIpLocation> {
        if !self.config.enabled {
            return None;
        }
        if is_lan_ip(ip) {
            return Some(GeoIpLocation {
                country: LAN_COUNTRY_CODE.to_string(),
                city: None,
                latitude: None,
                longitude: None,
            });
        }

        if self.config.provider == GeoIpProvider::Ipwhois {
            return self.ipwhois.lookup(ip).await;
        }

        let reader = {
            let guard = self.reader.read().await;
            guard.clone()
        }?;
        lookup_location(&reader, ip, self.config.edition)
    }

    fn uses_local_database(&self) -> bool {
        matches!(
            self.config.provider,
            GeoIpProvider::Dbip | GeoIpProvider::Custom
        )
    }

    async fn reload_from_disk(&self) -> bool {
        if !self.config.enabled {
            return false;
        }
        match maxminddb::Reader::open_readfile(&self.config.database_path) {
            Ok(reader) => {
                let mut guard = self.reader.write().await;
                *guard = Some(Arc::new(reader));
                info!(
                    path = %self.config.database_path.display(),
                    "geoip database loaded"
                );
                true
            }
            Err(error) => {
                warn!(
                    path = %self.config.database_path.display(),
                    error = ?error,
                    "geoip database is not available"
                );
                false
            }
        }
    }
}

impl IpwhoisClient {
    fn new(endpoint: impl Into<Arc<str>>) -> Self {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(IPWHOIS_TIMEOUT_SECS))
            .build()
        {
            Ok(client) => Some(client),
            Err(error) => {
                warn!(error = ?error, "failed to build ipwhois client");
                None
            }
        };

        Self {
            client,
            endpoint: endpoint.into(),
            cache: Arc::new(RwLock::new(HashMap::new())),
            retry_after: Arc::new(RwLock::new(None)),
        }
    }

    async fn lookup(&self, ip: IpAddr) -> Option<GeoIpLocation> {
        if let Some(location) = self.cached_location(ip).await {
            return Some(location);
        }
        if self.is_rate_limited().await {
            return None;
        }

        let client = self.client.as_ref()?;
        let url = match ipwhois_lookup_url(self.endpoint.as_ref(), ip) {
            Ok(url) => url,
            Err(error) => {
                warn!(error = ?error, "failed to build ipwhois lookup url");
                return None;
            }
        };
        let response = match client.get(url).send().await {
            Ok(response) => response,
            Err(_) => {
                warn!("ipwhois lookup request failed");
                return None;
            }
        };
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            self.set_retry_after(response.headers()).await;
            warn!("ipwhois rate limit reached; geoip lookup will retry later");
            return None;
        }
        let response = match response.error_for_status() {
            Ok(response) => response,
            Err(error) => {
                warn!(
                    status = error.status().map(|status| status.as_u16()),
                    "ipwhois lookup returned an error status"
                );
                return None;
            }
        };
        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => {
                warn!(error = ?error, "failed to read ipwhois lookup response");
                return None;
            }
        };
        let payload = match serde_json::from_slice::<IpwhoisResponse>(&body) {
            Ok(payload) => payload,
            Err(error) => {
                warn!(error = ?error, "failed to decode ipwhois lookup response");
                return None;
            }
        };
        let location = ipwhois_location_from_response(payload)?;
        self.cache_location(ip, location.clone()).await;
        Some(location)
    }

    async fn cached_location(&self, ip: IpAddr) -> Option<GeoIpLocation> {
        let now = Instant::now();
        let guard = self.cache.read().await;
        let cached = guard.get(&ip)?;
        (cached.expires_at > now).then(|| cached.location.clone())
    }

    async fn cache_location(&self, ip: IpAddr, location: GeoIpLocation) {
        let now = Instant::now();
        let mut guard = self.cache.write().await;
        if !guard.contains_key(&ip) && guard.len() >= IPWHOIS_CACHE_MAX_ENTRIES {
            prune_ipwhois_cache(&mut guard, now);
        }
        guard.insert(
            ip,
            CachedIpwhoisLocation {
                location,
                expires_at: now + Duration::from_secs(IPWHOIS_CACHE_TTL_SECS),
            },
        );
    }

    async fn is_rate_limited(&self) -> bool {
        let now = Instant::now();
        {
            let guard = self.retry_after.read().await;
            match *guard {
                Some(until) if until > now => return true,
                None => return false,
                Some(_) => {}
            }
        }

        let mut guard = self.retry_after.write().await;
        match *guard {
            Some(until) if until > now => true,
            Some(_) => {
                *guard = None;
                false
            }
            None => false,
        }
    }

    async fn set_retry_after(&self, headers: &reqwest::header::HeaderMap) {
        let duration = headers
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(IPWHOIS_RETRY_AFTER_FALLBACK_SECS));
        let mut guard = self.retry_after.write().await;
        *guard = Some(Instant::now() + duration);
    }
}

fn prune_ipwhois_cache(cache: &mut HashMap<IpAddr, CachedIpwhoisLocation>, now: Instant) {
    cache.retain(|_, cached| cached.expires_at > now);
    while cache.len() >= IPWHOIS_CACHE_MAX_ENTRIES {
        let Some(ip) = cache.keys().next().copied() else {
            break;
        };
        cache.remove(&ip);
    }
}

fn should_skip_download(config: &GeoIpConfig) -> bool {
    !config.auto_update
        || config.provider != GeoIpProvider::Dbip
        || (database_is_fresh(&config.database_path, config.update_interval_days)
            && database_matches_edition(&config.database_path, config.edition))
}

fn ipwhois_lookup_url(endpoint: &str, ip: IpAddr) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(endpoint).context("parse ipwhois endpoint")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("ipwhois endpoint cannot be a base URL"))?
        .pop_if_empty()
        .push(&ip.to_string());
    url.query_pairs_mut().append_pair("fields", IPWHOIS_FIELDS);
    Ok(url)
}

fn ipwhois_location_from_response(response: IpwhoisResponse) -> Option<GeoIpLocation> {
    if !response.success {
        return None;
    }
    let country = clean_location_text(response.country_code)
        .map(|code| code.to_ascii_uppercase())
        .or_else(|| clean_location_text(response.country))?;
    let city = clean_location_text(response.city).or_else(|| clean_location_text(response.region));
    let (latitude, longitude) = sanitize_ipwhois_coordinates(response.latitude, response.longitude);
    sanitize_location_override(Some(country), city, latitude, longitude)
        .ok()
        .flatten()
}

fn clean_location_text(value: Option<String>) -> Option<String> {
    let value = value?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn sanitize_ipwhois_coordinates(
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> (Option<f64>, Option<f64>) {
    match (latitude, longitude) {
        (Some(latitude), Some(longitude))
            if latitude.is_finite()
                && longitude.is_finite()
                && (-90.0..=90.0).contains(&latitude)
                && (-180.0..=180.0).contains(&longitude) =>
        {
            (Some(latitude), Some(longitude))
        }
        _ => (None, None),
    }
}

fn database_is_fresh(path: &Path, update_interval_days: u64) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let Ok(age) = modified.elapsed() else {
        return false;
    };
    age.as_secs() < update_interval_days.saturating_mul(24 * 60 * 60)
}

fn database_matches_edition(path: &Path, edition: GeoIpEdition) -> bool {
    let Ok(reader) = maxminddb::Reader::open_readfile(path) else {
        return false;
    };
    database_type_matches_edition(&reader.metadata.database_type, edition)
}

fn database_type_matches_edition(database_type: &str, edition: GeoIpEdition) -> bool {
    let database_type = database_type.to_ascii_lowercase();
    match edition {
        GeoIpEdition::CountryLite => database_type.contains("country"),
        GeoIpEdition::CityLite => database_type.contains("city"),
    }
}

async fn download_dbip_database(config: &GeoIpConfig) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("build geoip download client")?;
    let now = chrono::Utc::now();

    for url in dbip_download_urls(
        config.edition,
        now.year(),
        now.month(),
        DBIP_DOWNLOAD_ATTEMPTS,
    ) {
        let response = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("download DB-IP Lite database from {url}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            info!(url = %url, "DB-IP Lite database is not published for this month, trying previous release");
            continue;
        }

        let response = response
            .error_for_status()
            .with_context(|| format!("DB-IP Lite download returned an error status for {url}"))?;
        let compressed = response
            .bytes()
            .await
            .context("read DB-IP Lite download body")?;
        let mut decoder = GzDecoder::new(compressed.as_ref());
        let mut database = Vec::new();
        decoder
            .read_to_end(&mut database)
            .context("decompress DB-IP Lite database")?;
        replace_database(&config.database_path, database).await?;
        return Ok(());
    }

    anyhow::bail!(
        "DB-IP Lite database was not found in the last {DBIP_DOWNLOAD_ATTEMPTS} monthly releases"
    );
}

async fn replace_database(path: &Path, database: Vec<u8>) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create geoip directory {}", parent.display()))?;
    }
    let temp_path = temporary_database_path(path);
    tokio::fs::write(&temp_path, database)
        .await
        .with_context(|| format!("write temporary geoip database {}", temp_path.display()))?;
    tokio::fs::rename(&temp_path, path).await.with_context(|| {
        format!(
            "replace geoip database {} with {}",
            path.display(),
            temp_path.display(),
        )
    })?;
    Ok(())
}

fn temporary_database_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "geoip.mmdb".into());
    name.push(".tmp");
    path.with_file_name(name)
}

fn dbip_download_urls(
    edition: GeoIpEdition,
    year: i32,
    month: u32,
    attempts: usize,
) -> Vec<String> {
    let mut year = year;
    let mut month = month;
    let mut urls = Vec::with_capacity(attempts);

    for _ in 0..attempts {
        urls.push(dbip_download_url(edition, year, month));
        (year, month) = previous_year_month(year, month);
    }

    urls
}

fn previous_year_month(year: i32, month: u32) -> (i32, u32) {
    if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    }
}

fn dbip_download_url(edition: GeoIpEdition, year: i32, month: u32) -> String {
    let suffix = match edition {
        GeoIpEdition::CountryLite => "country-lite",
        GeoIpEdition::CityLite => "city-lite",
    };
    format!(
        "https://download.db-ip.com/free/dbip-{suffix}-{}-{:02}.mmdb.gz",
        year, month,
    )
}

fn lookup_location(
    reader: &maxminddb::Reader<Vec<u8>>,
    ip: IpAddr,
    edition: GeoIpEdition,
) -> Option<GeoIpLocation> {
    match edition {
        GeoIpEdition::CountryLite => lookup_country(reader, ip),
        GeoIpEdition::CityLite => lookup_city(reader, ip),
    }
}

fn lookup_country(reader: &maxminddb::Reader<Vec<u8>>, ip: IpAddr) -> Option<GeoIpLocation> {
    let country = reader.lookup(ip).ok()?.decode::<geoip2::Country>().ok()??;
    let iso_code = country
        .country
        .iso_code
        .or(country.registered_country.iso_code)?;
    Some(GeoIpLocation {
        country: iso_code.to_ascii_uppercase(),
        city: None,
        latitude: None,
        longitude: None,
    })
}

fn lookup_city(reader: &maxminddb::Reader<Vec<u8>>, ip: IpAddr) -> Option<GeoIpLocation> {
    let city = reader.lookup(ip).ok()?.decode::<geoip2::City>().ok()??;
    let country = city.country.iso_code.or(city.registered_country.iso_code)?;
    let city_name = city
        .city
        .names
        .english
        .or(city.city.names.simplified_chinese)
        .or(city.city.names.french)
        .or(city.city.names.spanish)
        .or(city.city.names.japanese)
        .or(city.city.names.german)
        .or(city.city.names.brazilian_portuguese)
        .or(city.city.names.russian)
        .map(str::to_string);
    Some(GeoIpLocation {
        country: country.to_ascii_uppercase(),
        city: city_name,
        latitude: city.location.latitude,
        longitude: city.location.longitude,
    })
}

fn is_lan_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_lan_ipv4(ip),
        IpAddr::V6(ip) => is_lan_ipv6(ip),
    }
}

fn is_lan_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || octets[0] == 0
}

fn is_lan_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || matches!(ip.segments(), [0x2001, 0x0db8, ..])
}

#[cfg(test)]
mod cache_tests;

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nodelite_proto::{GeoIpConfig, GeoIpEdition, GeoIpProvider};

    use std::path::Path;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{
        GeoIpResolver, IpwhoisResponse, database_type_matches_edition, dbip_download_url,
        dbip_download_urls, ipwhois_location_from_response, is_lan_ip, temporary_database_path,
    };

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
}
