//! CLI entry point; existing Agent installations are never enrolled automatically.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = nodelite_updater::run().await {
        eprintln!("nodelite-updater: {error}");
        std::process::exit(1);
    }
}
