//! Each invocation runs one named workload in its own process for comparable resource baselines.

#[path = "load/mod.rs"]
mod load;

const SCENARIOS: &[&str] = &[
    "scaling",
    "api-surface",
    "reconnect",
    "token-budget",
    "large-fleet",
    "dashboard",
    "history-pressure",
    "history-matrix",
    "payload",
    "read-write",
];

fn main() -> anyhow::Result<()> {
    let scenario = std::env::args().skip(1).find(|arg| arg != "--bench");
    let Some(scenario) = scenario.filter(|arg| arg != "--list" && arg != "--help") else {
        println!(
            "cargo bench -p nodelite-server --features bench-internals --bench load -- <scenario>"
        );
        println!("Scenarios: {}", SCENARIOS.join(", "));
        return Ok(());
    };
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()?
        .block_on(load::run(&scenario))
}
