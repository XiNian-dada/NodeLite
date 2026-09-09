//! Backup inventory follows the real config parser, including external and relative paths.

use super::*;
use crate::test_support::test_server_config;

#[test]
fn manifest_contains_all_persistent_files_and_sqlite_sidecars_without_credentials() {
    let mut config = test_server_config(
        "0.0.0.0:8080".parse().expect("listen address"),
        "https://monitor.example.com".into(),
        "config/nodes.json".into(),
        "/srv/external/history.sqlite3".into(),
        "data/snapshot.json".into(),
    );
    config.audit.db_path = PathBuf::from("/srv/audit events.sqlite3");
    config.geoip.database_path = PathBuf::from("data/geoip.mmdb");
    let manifest = render_manifest(
        Path::new("config/server.toml"),
        &config,
        Path::new("/opt/nodelite"),
    )
    .expect("render metadata");
    assert!(manifest.starts_with("nodelite-upgrade-manifest-v1\n"));
    assert!(manifest.contains("ready_url=http://127.0.0.1:8080/readyz\n"));
    for path in [
        "/opt/nodelite/config/server.toml",
        "/opt/nodelite/config/nodes.json",
        "/opt/nodelite/data/snapshot.json",
        "/opt/nodelite/data/geoip.mmdb",
        "/srv/external/history.sqlite3",
        "/srv/audit events.sqlite3",
    ] {
        assert!(
            manifest.contains(&format!("path={path}\n")),
            "missing {path}"
        );
    }
    for db in ["/srv/external/history.sqlite3", "/srv/audit events.sqlite3"] {
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(manifest.contains(&format!("path={db}{suffix}\n")));
        }
    }
    assert_eq!(
        manifest
            .lines()
            .filter(|line| line.starts_with("path="))
            .count(),
        12
    );
    assert!(!manifest.contains("password"));
    assert!(!manifest.contains("token"));
    assert!(!manifest.contains("username"));
    if let Some(auth) = &config.readonly_auth {
        assert!(!manifest.contains(&auth.password));
    }
}

#[test]
fn probe_addresses_cover_ipv4_ipv6_and_explicit_bind_addresses() {
    for (listen, expected) in [
        ("0.0.0.0:80", "http://127.0.0.1:80/readyz"),
        ("[::]:80", "http://[::1]:80/readyz"),
        ("[::1]:8080", "http://[::1]:8080/readyz"),
        ("192.0.2.10:8080", "http://192.0.2.10:8080/readyz"),
    ] {
        assert_eq!(
            readiness_url(listen.parse().expect("listen address")),
            expected
        );
    }
}

#[test]
fn line_breaks_in_a_backup_path_are_rejected_before_output() {
    for value in ["data/line\nbreak", "data/line\rbreak", "data/nul\0byte"] {
        assert!(backup_paths(Path::new(value), Path::new("/opt/nodelite")).is_err());
    }
}

#[cfg(unix)]
#[test]
fn database_symlinks_resolve_to_the_actual_file_before_sidecars_are_derived() {
    let dir = temporary_directory();
    let database = dir.join("history.sqlite3");
    std::fs::write(&database, "old database").expect("database fixture");
    let link = dir.join("db-link");
    std::os::unix::fs::symlink(&database, &link).expect("database link");
    assert_eq!(
        backup_paths(&link, &dir).expect("configured and resolved backup paths"),
        [link, database.canonicalize().expect("canonical fixture")]
    );
    std::fs::remove_dir_all(dir).expect("remove temporary directory");
}

#[cfg(unix)]
#[test]
fn manifest_preserves_configured_symlinks_alongside_targets() {
    let dir = temporary_directory();
    let entries = dir.join("entries");
    let targets = dir.join("targets");
    std::fs::create_dir(&entries).expect("entry directory");
    std::fs::create_dir(&targets).expect("target directory");
    std::os::unix::fs::symlink(&entries, dir.join("linked")).expect("directory link");
    let filenames = ["server.toml", "nodes.json", "snapshot.json", "geoip.mmdb"];
    for filename in filenames {
        std::fs::write(targets.join(filename), "before upgrade").expect("file fixture");
        std::os::unix::fs::symlink(
            Path::new("../targets").join(filename),
            entries.join(filename),
        )
        .expect("relative file link");
    }
    let mut config = test_server_config(
        "127.0.0.1:8080".parse().expect("listen address"),
        "https://monitor.example.com".into(),
        "linked/nodes.json".into(),
        "history.sqlite3".into(),
        "linked/snapshot.json".into(),
    );
    config.geoip.database_path = PathBuf::from("linked/geoip.mmdb");
    let manifest = render_manifest(Path::new("linked/server.toml"), &config, &dir)
        .expect("render symlink inventory");
    std::fs::remove_dir_all(&dir).expect("remove temporary directory");
    for filename in filenames {
        for path in [dir.join("linked").join(filename), targets.join(filename)] {
            assert!(
                manifest
                    .lines()
                    .any(|line| line == format!("path={}", path.display())),
                "missing backup path {}",
                path.display(),
            );
        }
    }
}

#[cfg(unix)]
fn temporary_directory() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "nodelite-upgrade-symlink-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).expect("temporary directory");
    dir.canonicalize().expect("canonical temporary directory")
}
