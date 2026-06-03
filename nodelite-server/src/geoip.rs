//! GeoIP lookup and DB-IP Lite database preparation.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Datelike;
use flate2::read::GzDecoder;
use maxminddb::geoip2;
use nodelite_proto::{GeoIpConfig, GeoIpEdition, GeoIpLocation, GeoIpProvider};
use tokio::sync::RwLock;
use tracing::{info, warn};

const LAN_COUNTRY_CODE: &str = "LAN";
const DOWNLOAD_TIMEOUT_SECS: u64 = 30;

type GeoIpReader = Arc<maxminddb::Reader<Vec<u8>>>;

#[derive(Clone)]
pub(crate) struct GeoIpResolver {
    config: GeoIpConfig,
    reader: Arc<RwLock<Option<GeoIpReader>>>,
}

impl GeoIpResolver {
    pub(crate) async fn new(config: GeoIpConfig) -> Self {
        let resolver = Self {
            config,
            reader: Arc::new(RwLock::new(None)),
        };
        resolver.reload_from_disk().await;
        resolver
    }

    pub(crate) async fn prepare_database(&self) -> bool {
        if !self.config.enabled {
            return false;
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

        let reader = {
            let guard = self.reader.read().await;
            guard.clone()
        }?;
        lookup_location(&reader, ip, self.config.edition)
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

fn should_skip_download(config: &GeoIpConfig) -> bool {
    !config.auto_update
        || config.provider == GeoIpProvider::Custom
        || database_is_fresh(&config.database_path, config.update_interval_days)
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

async fn download_dbip_database(config: &GeoIpConfig) -> Result<()> {
    let url = dbip_download_url(config.edition);
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("build geoip download client")?
        .get(url)
        .send()
        .await
        .context("download DB-IP Lite database")?
        .error_for_status()
        .context("DB-IP Lite download returned an error status")?;
    let compressed = response
        .bytes()
        .await
        .context("read DB-IP Lite download body")?;
    let mut decoder = GzDecoder::new(compressed.as_ref());
    let mut database = Vec::new();
    decoder
        .read_to_end(&mut database)
        .context("decompress DB-IP Lite database")?;
    if let Some(parent) = config.database_path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create geoip directory {}", parent.display()))?;
    }
    let temp_path = temporary_database_path(&config.database_path);
    tokio::fs::write(&temp_path, database)
        .await
        .with_context(|| format!("write temporary geoip database {}", temp_path.display()))?;
    tokio::fs::rename(&temp_path, &config.database_path)
        .await
        .with_context(|| {
            format!(
                "replace geoip database {} with {}",
                config.database_path.display(),
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

fn dbip_download_url(edition: GeoIpEdition) -> String {
    let now = chrono::Utc::now();
    let suffix = match edition {
        GeoIpEdition::CountryLite => "country-lite",
        GeoIpEdition::CityLite => "city-lite",
    };
    format!(
        "https://download.db-ip.com/free/dbip-{suffix}-{}-{:02}.mmdb.gz",
        now.year(),
        now.month(),
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
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.octets()[0] == 0
}

fn is_lan_ipv6(ip: Ipv6Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || matches!(ip.segments(), [0x2001, 0x0db8, ..])
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use nodelite_proto::GeoIpEdition;

    use std::path::Path;

    use super::{dbip_download_url, is_lan_ip, temporary_database_path};

    #[test]
    fn lan_ip_detection_covers_private_and_documentation_ranges() {
        for value in [
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.10",
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

    #[test]
    fn dbip_download_url_uses_requested_edition() {
        assert!(dbip_download_url(GeoIpEdition::CountryLite).contains("dbip-country-lite-"));
        assert!(dbip_download_url(GeoIpEdition::CityLite).contains("dbip-city-lite-"));
    }

    #[test]
    fn temporary_database_path_uses_sibling_tmp_file() {
        assert_eq!(
            temporary_database_path(Path::new("/var/lib/nodelite/geoip/dbip.mmdb")),
            Path::new("/var/lib/nodelite/geoip/dbip.mmdb.tmp"),
        );
    }
}
