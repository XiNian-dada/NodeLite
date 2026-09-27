//! NodeLite 服务端二进制主入口。

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    nodelite_server::cli_main()
        .await
        .map_err(anyhow::Error::new)
}
