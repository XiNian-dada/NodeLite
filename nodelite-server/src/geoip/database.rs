//! DB-IP database freshness, monthly download fallback, and atomic replacement.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Datelike;
use flate2::read::GzDecoder;
use tracing::info;

use nodelite_proto::{GeoIpConfig, GeoIpEdition, GeoIpProvider};

const DOWNLOAD_TIMEOUT_SECS: u64 = 30;
const DBIP_DOWNLOAD_ATTEMPTS: usize = 12;

pub(super) fn should_skip_download(config: &GeoIpConfig) -> bool {
    !config.auto_update
        || config.provider != GeoIpProvider::Dbip
        || (database_is_fresh(&config.database_path, config.update_interval_days)
            && database_matches_edition(&config.database_path, config.edition))
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

pub(super) fn database_type_matches_edition(database_type: &str, edition: GeoIpEdition) -> bool {
    let database_type = database_type.to_ascii_lowercase();
    match edition {
        GeoIpEdition::CountryLite => database_type.contains("country"),
        GeoIpEdition::CityLite => database_type.contains("city"),
    }
}

pub(super) async fn download_dbip_database(config: &GeoIpConfig) -> Result<()> {
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

pub(super) fn temporary_database_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "geoip.mmdb".into());
    name.push(".tmp");
    path.with_file_name(name)
}

pub(super) fn dbip_download_urls(
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

pub(super) fn dbip_download_url(edition: GeoIpEdition, year: i32, month: u32) -> String {
    let suffix = match edition {
        GeoIpEdition::CountryLite => "country-lite",
        GeoIpEdition::CityLite => "city-lite",
    };
    format!(
        "https://download.db-ip.com/free/dbip-{suffix}-{}-{:02}.mmdb.gz",
        year, month,
    )
}
