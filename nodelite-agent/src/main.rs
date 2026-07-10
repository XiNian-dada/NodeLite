//! NodeLite Agent 入口程序。

use anyhow::Result;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    nodelite_agent::runtime::run().await
}
