use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Bytes;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ConnectTarget {
    Ip { ip: Ipv4Addr, port: u16 },
    Host { host: String, port: u16 },
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Connect { target: ConnectTarget },
    Udp,
}

fn parse_arg(flag: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return args[i + 1].clone();
        }
    }
    default.to_string()
}

fn resolve_target(target: ConnectTarget) -> Result<SocketAddr> {
    match target {
        ConnectTarget::Ip { ip, port } => Ok(SocketAddr::new(IpAddr::V4(ip), port)),
        ConnectTarget::Host { host, port } => format!("{}:{}", host, port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("DNS resolution failed: {}:{}", host, port)),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let verbose = std::env::args().any(|a| a == "-v");

    let default_level = if verbose { "info" } else { "warn" };

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", default_level);
    }
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(default_level),
    )
    .format_timestamp_secs()
    .init();

    let port: u16 = parse_arg("-p", "1092").parse()?;
    let bind = parse_arg("-b", "127.0.0.1");
    let shutdown_secs: u64 = parse_arg("-t", "60").parse().unwrap_or(60);

    let listener = TcpListener::bind(format!("{}:{}", bind, port)).await?;
    log::info!("WebSocket proxy started on {}:{}", bind, port);
    log::info!("Auto-shutdown after {}s with no client", shutdown_secs);

    let mut conn_id: u32 = 0;
    // Active connection count. When it reaches 0 and no new client
    // connects within `shutdown_secs`, the proxy exits.
    let active = Arc::new(AtomicUsize::new(0));

    loop {
        match tokio::time::timeout(Duration::from_secs(shutdown_secs), listener.accept()).await {
            Ok(Ok((stream, addr))) => {
                conn_id = conn_id.wrapping_add(1);
                let id = conn_id;
                let active_clone = active.clone();
                active.fetch_add(1, Ordering::SeqCst);
                log::info!("[{}] New client: {}", id, addr.ip());
                tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    let result = handle_client(stream, id).await;
                    match result {
                        Ok(()) => log::info!("[{}] Connection closed ({:?})", id, start.elapsed()),
                        Err(e) => log::warn!("[{}] Error after {:?}: {}", id, start.elapsed(), e),
                    }
                    active_clone.fetch_sub(1, Ordering::SeqCst);
                    log::info!("[{}] Active connections: {}", id, active_clone.load(Ordering::SeqCst));
                });
            }
            Ok(Err(e)) => {
                log::error!("accept error: {}", e);
                return Err(e.into());
            }
            Err(_) => {
                // No new client within the timeout window.
                if active.load(Ordering::SeqCst) == 0 {
                    log::info!("No client for {}s — shutting down", shutdown_secs);
                    return Ok(());
                }
            }
        }
    }
}

async fn handle_client(stream: TcpStream, id: u32) -> Result<()> {
    stream.set_nodelay(true)?;
    let ws = accept_async(stream).await?;
    log::info!("[{}] WebSocket handshake ok", id);

    let (mut sink, mut source) = ws.split();

    let first = match source.next().await {
        Some(Ok(Message::Text(t))) => t.to_string(),
        Some(Ok(_)) => anyhow::bail!("expected JSON request as first message"),
        Some(Err(e)) => anyhow::bail!("read error: {}", e),
        None => anyhow::bail!("connection closed before request"),
    };

    let req: Request = match serde_json::from_str(&first) {
        Ok(r) => r,
        Err(e) => {
            let _ = sink
                .send(Message::Text(Utf8Bytes::from(format!(r#"{{"error":{:?}}}"#, e.to_string()))))
                .await;
            return Ok(());
        }
    };

    match req {
        Request::Connect { target } => {
            let target_addr = match resolve_target(target) {
                Ok(a) => a,
                Err(e) => {
                    let _ = sink
                        .send(Message::Text(Utf8Bytes::from(format!(r#"{{"error":{:?}}}"#, e.to_string()))))
                        .await;
                    return Ok(());
                }
            };
            log::info!("[{}] CONNECT -> {}", id, target_addr);

            let remote = match TcpStream::connect(target_addr).await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[{}] CONNECT failed {} -> {}", id, target_addr, e);
                    let _ = sink
                        .send(Message::Text(Utf8Bytes::from(format!(r#"{{"error":{:?}}}"#, e.to_string()))))
                        .await;
                    return Ok(());
                }
            };
            remote.set_nodelay(true)?;

            if sink.send(Message::Text(Utf8Bytes::from_static(r#"{"ok":true}"#))).await.is_err() {
                return Ok(());
            }
            log::info!("[{}] TCP connected: {}", id, target_addr);

            relay_tcp(sink, source, remote, id, target_addr).await?;
        }

        Request::Udp => {
            log::info!("[{}] UDP relay", id);
            if sink.send(Message::Text(Utf8Bytes::from_static(r#"{"ok":true}"#))).await.is_err() {
                return Ok(());
            }
            relay_udp(sink, source, id).await?;
        }
    }

    Ok(())
}

async fn relay_tcp(
    mut sink: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    mut source: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<TcpStream>,
    >,
    remote: TcpStream,
    id: u32,
    target_addr: SocketAddr,
) -> Result<()> {
    let (mut remote_r, mut remote_w) = remote.into_split();

    let ws_to_remote = async {
        let mut up: u64 = 0;
        while let Some(msg) = source.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    if remote_w.write_all(&data).await.is_err() {
                        break;
                    }
                    up += data.len() as u64;
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
        let _ = remote_w.shutdown().await;
        up
    };

    let remote_to_ws = async {
        let mut down: u64 = 0;
        let mut buf = [0u8; 65536];
        loop {
            match remote_r.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if sink.send(Message::Binary(Bytes::from(buf[..n].to_vec()))).await.is_err() {
                        break;
                    }
                    down += n as u64;
                }
            }
        }
        let _ = sink.close().await;
        down
    };

    let (up, down) = tokio::join!(ws_to_remote, remote_to_ws);
    log::info!(
        "[{}] TCP closed: {} | ↑{} ↓{} bytes",
        id, target_addr, up, down
    );
    Ok(())
}

/// UDP relay over one WebSocket.
///
/// Each binary WS message is one UDP datagram, framed with a 4-byte header
/// (`0,0,0,ATYP,addr,port`) so the target/source is carried per packet.
async fn relay_udp(
    mut sink: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    mut source: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<TcpStream>,
    >,
    id: u32,
) -> Result<()> {
    // flow_id -> upstream socket. One socket per client flow id (carried in
    // the RSV field); replies received on a socket are framed back with that
    // socket's id so the client can demux same-target flows.
    let pool: Arc<Mutex<HashMap<u16, Arc<UdpSocket>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (reply_tx, mut reply_rx) = mpsc::channel::<(u16, SocketAddr, Vec<u8>)>(2048);
    let pool_up = pool.clone();

    let to_target = async {
        let mut up_count: u64 = 0;
        let mut pkt_count: u64 = 0;
        while let Some(msg) = source.next().await {
            let Ok(Message::Binary(dgram)) = msg else { break };
            if dgram.len() < 10 {
                continue;
            }
            let flow_id = u16::from_be_bytes([dgram[0], dgram[1]]);
            let atyp = dgram[3];
            let (target, payload_start) = match atyp {
                0x01 => {
                    let ip = Ipv4Addr::new(dgram[4], dgram[5], dgram[6], dgram[7]);
                    let port = u16::from_be_bytes([dgram[8], dgram[9]]);
                    (SocketAddr::new(IpAddr::V4(ip), port), 10)
                }
                0x03 => {
                    if dgram.len() < 5 {
                        continue;
                    }
                    let dlen = dgram[4] as usize;
                    if dgram.len() < 5 + dlen + 2 {
                        continue;
                    }
                    let host = match std::str::from_utf8(&dgram[5..5 + dlen]) {
                        Ok(h) => h,
                        Err(_) => continue,
                    };
                    let port = u16::from_be_bytes([dgram[5 + dlen], dgram[6 + dlen]]);
                    match format!("{}:{}", host, port).to_socket_addrs() {
                        Ok(mut a) => match a.next() {
                            Some(x) => (x, 5 + dlen + 2),
                            None => continue,
                        },
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };

            let sock = match get_flow_socket(&pool_up, flow_id, reply_tx.clone(), id).await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[{}] UDP flow {} bind failed: {}", id, flow_id, e);
                    continue;
                }
            };
            if let Ok(n) = sock.send_to(&dgram[payload_start..], target).await {
                up_count += n as u64;
                pkt_count += 1;
                if pkt_count == 1 || pkt_count % 1000 == 0 {
                    log::info!("[{}] UDP up pkt#{} fid={} -> {} ({} bytes)", id, pkt_count, flow_id, target, n);
                }
            }
        }
        (up_count, pkt_count)
    };

    let from_target = async {
        let mut down_count: u64 = 0;
        let mut pkt_count: u64 = 0;
        while let Some((flow_id, src, payload)) = reply_rx.recv().await {
            let (ip, port) = match src {
                SocketAddr::V4(v4) => (v4.ip().octets(), v4.port()),
                _ => continue,
            };
            let mut frame = Vec::with_capacity(10 + payload.len());
            frame.extend_from_slice(&flow_id.to_be_bytes());
            frame.push(0); // FRAG
            frame.push(0x01); // ATYP IPv4
            frame.extend_from_slice(&ip);
            frame.extend_from_slice(&port.to_be_bytes());
            frame.extend_from_slice(&payload);

            if sink.send(Message::Binary(Bytes::from(frame))).await.is_err() {
                break;
            }
            down_count += payload.len() as u64;
            pkt_count += 1;
            if pkt_count == 1 || pkt_count % 1000 == 0 {
                log::info!("[{}] UDP down pkt#{} fid={} <- {} ({} bytes)", id, pkt_count, flow_id, src, payload.len());
            }
        }
        (down_count, pkt_count)
    };

    let ((up, up_pkts), (down, down_pkts)) = tokio::join!(to_target, from_target);
    log::info!(
        "[{}] UDP relay ended | ↑{} ({}pkts) ↓{} ({}pkts)",
        id, up, up_pkts, down, down_pkts
    );
    Ok(())
}

/// Returns the upstream socket for `flow_id`, creating it (and its reader
/// task) on first use. Reader tasks exit when the reply channel closes —
/// i.e. when the relay ends — releasing their socket Arcs.
async fn get_flow_socket(
    pool: &Arc<Mutex<HashMap<u16, Arc<UdpSocket>>>>,
    flow_id: u16,
    reply_tx: mpsc::Sender<(u16, SocketAddr, Vec<u8>)>,
    id: u32,
) -> Result<Arc<UdpSocket>> {
    if let Some(s) = pool.lock().await.get(&flow_id) {
        return Ok(s.clone());
    }
    let s = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
    let mut p = pool.lock().await;
    if let Some(existing) = p.get(&flow_id) {
        return Ok(existing.clone()); // lost a creation race
    }
    p.insert(flow_id, s.clone());
    drop(p);

    let local_port = s.local_addr()?.port();
    log::info!("[{}] UDP flow socket bound: fid={} port={}", id, flow_id, local_port);
    let rx_sock = s.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        loop {
            match rx_sock.recv_from(&mut buf).await {
                Ok((n, src)) => {
                    if reply_tx.send((flow_id, src, buf[..n].to_vec())).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    Ok(s)
}
