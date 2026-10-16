//! End-to-end tests for the phone-side `ws-proxy` binary.
//!
//! These spawn the *real* binary (`CARGO_BIN_EXE_ws-proxy`) and drive it over a
//! real loopback WebSocket, exercising the two transports the tunnel relies on:
//!
//! * TCP `{"op":"connect" ...}` — the SOCKS-style request/reply handshake and
//!   the byte-stream bridge.
//! * UDP `{"op":"udp"}` — the SOCKS5-UDP-compatible datagram framing
//!   (`fid(2) FRAG(1) ATYP(1) addr port payload`).
//!
//! The UDP tests double as leak regression tests: **DNS** (UDP/53) and
//! **STUN/WebRTC** (UDP) datagrams must survive the round trip byte-for-byte,
//! and two concurrent flows to the *same* resolver must not steal each other's
//! answers (the flow-id demux).

use futures_util::{SinkExt, StreamExt};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::net::{TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ── process helpers ──────────────────────────────────────────────────────────

/// Kills the spawned proxy when the test finishes (pass or panic).
struct ProxyGuard(Child);

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    // Bind an ephemeral port, read it, then drop the listener to hand the port
    // to the child. Small race window, acceptable for tests.
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// Spawns `ws-proxy` on a free port and waits until it accepts connections.
async fn start_proxy() -> (ProxyGuard, u16) {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_ws-proxy"))
        .args(["-b", "127.0.0.1", "-p", &port.to_string(), "-t", "30"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn ws-proxy");
    let guard = ProxyGuard(child);

    for _ in 0..400 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return (guard, port);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("ws-proxy did not start on port {port}");
}

// ── WebSocket helpers ────────────────────────────────────────────────────────

async fn connect_ws(port: u16) -> Ws {
    let url = format!("ws://127.0.0.1:{port}/");
    let (ws, _) = connect_async(url).await.expect("ws connect failed");
    ws
}

async fn next_text(ws: &mut Ws) -> String {
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(t))) => return t.to_string(),
            Some(Ok(_)) => continue,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }
}

async fn next_binary(ws: &mut Ws) -> Vec<u8> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(d))) => return d.to_vec(),
            Some(Ok(_)) => continue,
            other => panic!("expected a binary frame, got {other:?}"),
        }
    }
}

// ── UDP framing helpers ──────────────────────────────────────────────────────

/// SOCKS5-UDP-style frame with an IPv4 `ATYP` (0x01). 10-byte header.
fn ipv4_frame(fid: u16, ip: [u8; 4], port: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(10 + payload.len());
    f.extend_from_slice(&fid.to_be_bytes());
    f.push(0); // FRAG
    f.push(0x01); // ATYP IPv4
    f.extend_from_slice(&ip);
    f.extend_from_slice(&port.to_be_bytes());
    f.extend_from_slice(payload);
    f
}

/// Frame with a hostname `ATYP` (0x03): `fid(2) FRAG(1) ATYP(1) len(1) host port payload`.
fn host_frame(fid: u16, host: &str, port: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&fid.to_be_bytes());
    f.push(0);
    f.push(0x03);
    f.push(host.len() as u8);
    f.extend_from_slice(host.as_bytes());
    f.extend_from_slice(&port.to_be_bytes());
    f.extend_from_slice(payload);
    f
}

struct Reply {
    fid: u16,
    src: SocketAddr,
    payload: Vec<u8>,
}

fn parse_reply(f: &[u8]) -> Reply {
    assert!(f.len() >= 10, "reply frame shorter than its header: {f:?}");
    assert_eq!(f[2], 0, "FRAG must be 0");
    assert_eq!(f[3], 0x01, "replies are always ATYP IPv4");
    Reply {
        fid: u16::from_be_bytes([f[0], f[1]]),
        src: SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(f[4], f[5], f[6], f[7])),
            u16::from_be_bytes([f[8], f[9]]),
        ),
        payload: f[10..].to_vec(),
    }
}

/// Spawns a loopback UDP echo server and returns its port.
async fn udp_echo() -> u16 {
    let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = s.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut b = [0u8; 2048];
        loop {
            match s.recv_from(&mut b).await {
                Ok((n, src)) => {
                    let _ = s.send_to(&b[..n], src).await;
                }
                Err(_) => break,
            }
        }
    });
    port
}

// ── TCP CONNECT transport ────────────────────────────────────────────────────

#[tokio::test]
async fn tcp_connect_round_trips_bytes() {
    // Real TCP echo server on loopback.
    let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = echo.accept().await.unwrap();
        let (mut r, mut w) = s.split();
        let _ = tokio::io::copy(&mut r, &mut w).await;
    });

    let (_guard, port) = start_proxy().await;
    let mut ws = connect_ws(port).await;

    let req = format!(
        r#"{{"op":"connect","target":{{"kind":"ip","ip":"127.0.0.1","port":{echo_port}}}}}"#
    );
    ws.send(Message::Text(req.into())).await.unwrap();
    assert!(
        next_text(&mut ws).await.contains("\"ok\":true"),
        "CONNECT must be acknowledged"
    );

    ws.send(Message::Binary(b"ping-through-proxy".to_vec().into()))
        .await
        .unwrap();
    let echoed = next_binary(&mut ws).await;
    assert_eq!(echoed, b"ping-through-proxy");
}

#[tokio::test]
async fn tcp_connect_reports_error_for_dead_target() {
    let (_guard, port) = start_proxy().await;
    let mut ws = connect_ws(port).await;

    // Nothing is listening on this port.
    let dead = free_port();
    let req = format!(
        r#"{{"op":"connect","target":{{"kind":"ip","ip":"127.0.0.1","port":{dead}}}}}"#
    );
    ws.send(Message::Text(req.into())).await.unwrap();
    let reply = next_text(&mut ws).await;
    assert!(
        reply.contains("error"),
        "unreachable target must yield an error, got: {reply}"
    );
}

// ── UDP transport: framing + leak coverage ───────────────────────────────────

#[tokio::test]
async fn udp_dns_query_is_relayed_byte_for_byte() {
    let echo = udp_echo().await;
    let (_guard, port) = start_proxy().await;
    let mut ws = connect_ws(port).await;

    // UDP associate handshake.
    ws.send(Message::Text(r#"{"op":"udp"}"#.into())).await.unwrap();
    assert!(next_text(&mut ws).await.contains("\"ok\":true"));

    // A minimal DNS query: transaction id, flags, one question. The resolver is
    // "the phone" (the loopback echo) so the whole point is that UDP/53 is
    // tunneled and comes back intact.
    let query: Vec<u8> = vec![
        0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // header
        0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm', 0x00, // qname
        0x00, 0x01, 0x00, 0x01, // A, IN
    ];
    let frame = ipv4_frame(0xABCD, [127, 0, 0, 1], echo, &query);
    ws.send(Message::Binary(frame.into())).await.unwrap();

    let reply = parse_reply(&next_binary(&mut ws).await);
    assert_eq!(reply.fid, 0xABCD, "flow id echoed");
    assert_eq!(reply.src.port(), echo);
    assert_eq!(reply.payload, query, "DNS query must not be altered");
}

#[tokio::test]
async fn udp_webrtc_stun_binding_request_round_trips() {
    let echo = udp_echo().await;
    let (_guard, port) = start_proxy().await;
    let mut ws = connect_ws(port).await;

    ws.send(Message::Text(r#"{"op":"udp"}"#.into())).await.unwrap();
    assert!(next_text(&mut ws).await.contains("\"ok\":true"));

    // STUN binding request: type 0x0001, length 0, magic cookie 0x2112A442,
    // then a 12-byte transaction id.
    let mut stun = vec![0x00, 0x01, 0x00, 0x00];
    stun.extend_from_slice(&0x2112A442u32.to_be_bytes());
    stun.extend_from_slice(&[0xAA; 12]);

    let frame = ipv4_frame(0x0101, [127, 0, 0, 1], echo, &stun);
    ws.send(Message::Binary(frame.into())).await.unwrap();

    let reply = parse_reply(&next_binary(&mut ws).await);
    assert_eq!(reply.fid, 0x0101);
    assert_eq!(reply.payload, stun, "STUN must survive the tunnel intact");
}

#[tokio::test]
async fn udp_host_atyp_frames_are_supported() {
    // The client may send a hostname target (ATYP 0x03); the proxy resolves it.
    let echo = udp_echo().await;
    let (_guard, port) = start_proxy().await;
    let mut ws = connect_ws(port).await;

    ws.send(Message::Text(r#"{"op":"udp"}"#.into())).await.unwrap();
    assert!(next_text(&mut ws).await.contains("\"ok\":true"));

    let frame = host_frame(7, "127.0.0.1", echo, b"host-atyp");
    ws.send(Message::Binary(frame.into())).await.unwrap();

    let reply = parse_reply(&next_binary(&mut ws).await);
    assert_eq!(reply.fid, 7);
    assert_eq!(reply.payload, b"host-atyp");
}

#[tokio::test]
async fn udp_flow_ids_demux_two_clients_of_same_resolver() {
    // Two flows (different flow ids) to the *same* echo server. Replies must
    // come back tagged with their own flow id and payload — no cross-talk.
    let echo = udp_echo().await;
    let (_guard, port) = start_proxy().await;
    let mut ws = connect_ws(port).await;

    ws.send(Message::Text(r#"{"op":"udp"}"#.into())).await.unwrap();
    assert!(next_text(&mut ws).await.contains("\"ok\":true"));

    ws.send(Message::Binary(ipv4_frame(1, [127, 0, 0, 1], echo, b"one").into()))
        .await
        .unwrap();
    ws.send(Message::Binary(ipv4_frame(2, [127, 0, 0, 1], echo, b"two").into()))
        .await
        .unwrap();

    let mut got = std::collections::BTreeMap::new();
    while got.len() < 2 {
        let r = parse_reply(&next_binary(&mut ws).await);
        got.insert(r.fid, r.payload);
    }
    assert_eq!(got.get(&1).map(Vec::as_slice), Some(&b"one"[..]));
    assert_eq!(got.get(&2).map(Vec::as_slice), Some(&b"two"[..]));
}
