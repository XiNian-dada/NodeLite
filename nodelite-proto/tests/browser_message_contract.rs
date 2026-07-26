//! Locks the browser WebSocket JSON contract to the fixture consumed by the Vue client tests.

use std::{env, fs, path::PathBuf};

use chrono::{TimeZone, Utc};
use nodelite_proto::{
    BrowserMessage, NodeListIdentity, NodeListItem, NodeListLoadAverage, NodeListMemoryUsage,
    NodeListSnapshot, OverviewData,
};
use serde::Serialize;

const UPDATE_FIXTURES_ENV: &str = "UPDATE_BROWSER_MESSAGE_FIXTURES";

#[derive(Serialize)]
struct BrowserMessageContract {
    server_to_browser: ServerToBrowserMessages,
    browser_to_server: BrowserToServerMessages,
}

#[derive(Serialize)]
struct ServerToBrowserMessages {
    initial_state: BrowserMessage,
    overview_update: BrowserMessage,
    node_upsert: BrowserMessage,
    node_removed: BrowserMessage,
    pong: BrowserMessage,
}

#[derive(Serialize)]
struct BrowserToServerMessages {
    ping: BrowserMessage,
}

#[test]
fn browser_message_fixture_matches_rust_serialization() {
    let expected = render_contract_fixture();
    let fixture_path = fixture_path();

    if env::var_os(UPDATE_FIXTURES_ENV).is_some() {
        let parent = fixture_path
            .parent()
            .expect("browser message fixture should have a parent directory");
        fs::create_dir_all(parent).expect("create browser message fixture directory");
        fs::write(&fixture_path, &expected).expect("write browser message contract fixture");
    }

    let actual = fs::read_to_string(&fixture_path).unwrap_or_else(|error| {
        panic!(
            "read browser message fixture at {}: {error}; regenerate with {UPDATE_FIXTURES_ENV}=1 cargo test -p nodelite-proto --test browser_message_contract",
            fixture_path.display()
        )
    });

    assert_eq!(
        actual, expected,
        "browser message contract changed; review both Rust and TypeScript types, then regenerate with {UPDATE_FIXTURES_ENV}=1 cargo test -p nodelite-proto --test browser_message_contract"
    );
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../nodelite-server/web/src/ws/__fixtures__/browser_messages.json")
}

fn render_contract_fixture() -> String {
    let contract = contract_fixture();
    let json = serde_json::to_string_pretty(&contract).expect("serialize browser message contract");
    format!("{json}\n")
}

fn contract_fixture() -> BrowserMessageContract {
    let generated_at = Utc
        .with_ymd_and_hms(2026, 7, 1, 12, 34, 56)
        .single()
        .expect("valid fixture timestamp");
    let overview = OverviewData {
        generated_at,
        total_nodes: 1,
        online_nodes: 1,
        offline_nodes: 0,
        total_rx_bytes: 123_456,
        total_tx_bytes: 654_321,
        current_rx_bytes_per_sec: 128.5,
        current_tx_bytes_per_sec: 256.25,
        average_latency_ms: Some(15.5),
    };
    let node = NodeListItem {
        identity: NodeListIdentity {
            node_id: "contract-node-01".to_string(),
            node_label: "Contract Node".to_string(),
            hostname: "contract.example".to_string(),
            tags: vec!["contract".to_string(), "edge".to_string()],
        },
        geoip_country: Some("Singapore".to_string()),
        geoip_city: Some("Singapore".to_string()),
        geoip_latitude: Some(1.3521),
        geoip_longitude: Some(103.8198),
        location_override_country: None,
        location_override_city: None,
        location_override_latitude: None,
        location_override_longitude: None,
        snapshot: Some(NodeListSnapshot {
            cpu_usage_percent: Some(37.5),
            load: NodeListLoadAverage { one: 0.75 },
            memory: NodeListMemoryUsage {
                total_bytes: 8_589_934_592,
                used_bytes: 3_221_225_472,
            },
        }),
        latency_ms: Some(12),
        online: true,
    };

    BrowserMessageContract {
        server_to_browser: ServerToBrowserMessages {
            initial_state: BrowserMessage::InitialState {
                generated_at,
                overview: overview.clone(),
                nodes: vec![node.clone()],
            },
            overview_update: BrowserMessage::OverviewUpdate {
                generated_at,
                overview,
            },
            node_upsert: BrowserMessage::NodeUpsert {
                generated_at,
                node: Box::new(node),
            },
            node_removed: BrowserMessage::NodeRemoved {
                generated_at,
                node_id: "contract-node-01".to_string(),
            },
            pong: BrowserMessage::Pong,
        },
        browser_to_server: BrowserToServerMessages {
            ping: BrowserMessage::Ping,
        },
    }
}
