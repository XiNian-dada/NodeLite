//! DB-IP 离线数据库保鲜、按月回退下载与原子更新替换模块。

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Datelike;
use flate2::read::GzDecoder;
use tracing::warn;

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

    let urls = dbip_download_urls(
        config.edition,
        now.year(),
        now.month(),
        DBIP_DOWNLOAD_ATTEMPTS,
    );
    download_from_urls(&client, config, &urls).await
}

async fn download_from_urls(
    client: &reqwest::Client,
    config: &GeoIpConfig,
    urls: &[String],
) -> Result<()> {
    for url in urls {
        match download_month(client, config, url).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                warn!(error = ?error, "DB-IP download failed; trying previous monthly release")
            }
        }
    }
    anyhow::bail!("no DB-IP Lite monthly release could be downloaded")
}

async fn download_month(client: &reqwest::Client, config: &GeoIpConfig, url: &str) -> Result<()> {
    let compressed = client
        .get(url)
        .send()
        .await
        .context("download DB-IP Lite database")?
        .error_for_status()
        .context("DB-IP Lite download status")?
        .bytes()
        .await
        .context("read DB-IP Lite download body")?;
    let mut decoder = GzDecoder::new(compressed.as_ref());
    let mut database = Vec::new();
    decoder
        .read_to_end(&mut database)
        .context("decompress DB-IP Lite database")?;
    replace_database(&config.database_path, database).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn monthly_fallback_recovers_from_status_and_gzip_errors() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(b"fixture database").expect("gzip input");
        let compressed = gzip.finish().expect("gzip body");
        let stub = tokio::spawn(async move {
            for (status, body) in [
                ("503 Service Unavailable", Vec::new()),
                ("200 OK", b"invalid gzip".to_vec()),
                ("200 OK", compressed),
            ] {
                let (mut stream, _) = listener.accept().await.expect("request");
                let mut request = [0; 2048];
                let bytes = stream.read(&mut request).await.expect("read");
                assert!(bytes > 0, "stub received an HTTP request");
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .expect("headers");
                stream.write_all(&body).await.expect("body");
            }
        });
        let dir = std::env::temp_dir().join(format!(
            "nodelite-geoip-fallback-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().expect("timestamp")
        ));
        let mut config = crate::test_support::test_server_config(
            "127.0.0.1:8080".parse().expect("listen"),
            "http://localhost".into(),
            dir.join("registry"),
            dir.join("history"),
            dir.join("snapshot"),
        )
        .geoip;
        config.database_path = dir.join("geoip.mmdb");
        let urls = (0..3).map(|n| format!("{base}/{n}")).collect::<Vec<_>>();
        download_from_urls(&reqwest::Client::new(), &config, &urls)
            .await
            .expect("fallback succeeds");
        assert_eq!(
            std::fs::read(&config.database_path).expect("database"),
            b"fixture database"
        );
        stub.await.expect("stub finishes");
        std::fs::remove_file(config.database_path).expect("remove fixture");
        std::fs::remove_dir(dir).expect("remove fixture directory");
    }
}
