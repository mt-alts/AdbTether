use anyhow::Result;
use tokio_tungstenite::connect_async;

/// Checks whether the WebSocket proxy is alive.
///
/// Performs a real WebSocket handshake with the proxy. A successful handshake
/// means the proxy is listening and speaks WebSocket.
pub async fn proxy_alive(host: &str, port: u16) -> Result<()> {
    let url = format!("ws://{}:{}/", host, port);
    let (mut ws, _) = connect_async(url)
        .await
        .map_err(|e| anyhow::anyhow!("could not connect to proxy (is it running?): {e}"))?;
    let _ = ws.close(None).await;
    Ok(())
}
