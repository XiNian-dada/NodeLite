//! GeoIP lookup, ipwhois online resolution, and DB-IP Lite database preparation.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use maxminddb::geoip2;
use nodelite_proto::{GeoIpConfig, GeoIpEdition, GeoIpLocation, GeoIpProvider};
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

mod database;

use self::database::{download_dbip_database, should_skip_download};
use crate::sanitize::sanitize_location_override;

const LAN_COUNTRY_CODE: &str = "LAN";
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
    sanitize_location_override(Some(iso_code.to_ascii_uppercase()), None, None, None)
        .ok()
        .flatten()
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
    sanitize_location_override(
        Some(country.to_ascii_uppercase()),
        city_name,
        city.location.latitude,
        city.location.longitude,
    )
    .ok()
    .flatten()
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
mod tests;
