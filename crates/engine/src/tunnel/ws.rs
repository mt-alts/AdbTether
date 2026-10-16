use anyhow::Result;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Bytes;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tokio_tungstenite::{client_async, tungstenite::Message, WebSocketStream};

type Ws = WebSocketStream<TcpStream>;
type WsSink = futures_util::stream::SplitSink<Ws, Message>;
type WsSource = futures_util::stream::SplitStream<Ws>;

/// WebSocket connection exposed as an `AsyncRead + AsyncWrite` byte stream.
///
/// `connect_tcp` sends the CONNECT request, then wraps the WS in this adapter
/// so the tunnel can `split()` it like a plain TCP stream.
pub struct WsStream {
    sink: WsSink,
    source: WsSource,
    pending: Vec<u8>,
    pos: usize,
}

/// Dial the proxy with Nagle disabled. The relay carries many small request
/// frames; with Nagle plus delayed ACK each one can stall for tens of
/// milliseconds, which is exactly what makes browsing feel slow. The phone
/// side already sets this (`ws-proxy`), the engine side did not.
async fn dial(proxy: SocketAddr) -> Result<TcpStream> {
    let tcp = TcpStream::connect(proxy).await?;
    tcp.set_nodelay(true)?;
    Ok(tcp)
}

impl WsStream {
    async fn open(proxy: SocketAddr) -> Result<WsStream> {
        let url = format!("ws://{}/", proxy);
        let (ws, _) = client_async(url, dial(proxy).await?).await?;
        let (sink, source) = ws.split();
        Ok(WsStream {
            sink,
            source,
            pending: Vec::new(),
            pos: 0,
        })
    }

    async fn send_text(&mut self, text: &str) -> Result<()> {
        self.sink
            .send(Message::Text(Utf8Bytes::from(text.to_string())))
            .await?;
        Ok(())
    }

    async fn recv_text(&mut self) -> Result<String> {
        loop {
            match self.source.next().await {
                Some(Ok(Message::Text(t))) => return Ok(t.to_string()),
                Some(Ok(_)) => continue,
                Some(Err(e)) => anyhow::bail!("WebSocket error: {e}"),
                None => anyhow::bail!("WebSocket closed"),
            }
        }
    }
}

impl AsyncRead for WsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        if me.pos >= me.pending.len() {
            match Pin::new(&mut me.source).poll_next(cx) {
                Poll::Ready(Some(Ok(Message::Binary(data)))) => {
                    me.pending = data.to_vec();
                    me.pos = 0;
                }
                Poll::Ready(Some(Ok(_))) => return Poll::Ready(Ok(())),
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    )))
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
        let n = (me.pending.len() - me.pos).min(buf.remaining());
        buf.put_slice(&me.pending[me.pos..me.pos + n]);
        me.pos += n;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for WsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        let msg = Message::Binary(Bytes::from(buf.to_vec()));
        match Pin::new(&mut me.sink).poll_ready(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )))
            }
            Poll::Pending => return Poll::Pending,
        }
        match Pin::new(&mut me.sink).start_send(msg) {
            Ok(()) => {}
            Err(e) => {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )))
            }
        }
        // The frame is now queued in the sink, so report it as fully consumed.
        // Flushing is driven separately by `poll_flush` (the writer task calls
        // it). Returning `Pending` here after `start_send` would make the
        // caller re-submit the same buffer on the next poll and the frame
        // would be written twice, corrupting the tunneled byte stream.
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        match Pin::new(&mut me.sink).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        match Pin::new(&mut me.sink).poll_close(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// WebSocket CONNECT + byte stream.
pub async fn connect_tcp(proxy: SocketAddr, target: SocketAddr) -> Result<WsStream> {
    let mut ws = WsStream::open(proxy).await?;
    let req = format!(
        r#"{{"op":"connect","target":{{"kind":"ip","ip":"{}","port":{}}}}}"#,
        target.ip(),
        target.port()
    );
    ws.send_text(&req).await?;
    let resp = ws.recv_text().await?;
    if !resp.contains("\"ok\"") {
        anyhow::bail!("connect rejected: {resp}");
    }
    Ok(ws)
}

/// UDP relay over a single WebSocket.
///
/// Each binary WS message is one UDP datagram: a 4-byte header
/// (`0,0,0,ATYP,addr,port`) followed by the payload. The server resolves the
/// per-packet target/source from that header.
pub struct UdpAssociate {
    sink: Mutex<WsSink>,
    source: Mutex<WsSource>,
}

impl UdpAssociate {
    pub async fn bind(proxy: SocketAddr) -> Result<Self> {
        let url = format!("ws://{}/", proxy);
        let (ws, _) = client_async(url, dial(proxy).await?).await?;
        let (sink, source) = ws.split();
        let assoc = UdpAssociate {
            sink: Mutex::new(sink),
            source: Mutex::new(source),
        };
        assoc
            .sink
            .lock()
            .await
            .send(Message::Text(Utf8Bytes::from_static(r#"{"op":"udp"}"#)))
            .await?;
        Ok(assoc)
    }

    pub async fn udp_send(&self, target: SocketAddr, payload: &[u8], flow_id: u16) -> Result<()> {
        let mut dgram = Vec::with_capacity(10 + payload.len());
        // RSV carries the client-assigned flow id; the server echoes it back
        // in reply frames so multiple local sockets to the same target can be
        // demuxed (old servers send 0 -> legacy single-flow behaviour).
        dgram.extend_from_slice(&flow_id.to_be_bytes());
        dgram.push(0); // FRAG
        dgram.push(0x01); // ATYP IPv4
        match target {
            SocketAddr::V4(v4) => dgram.extend_from_slice(&v4.ip().octets()),
            SocketAddr::V6(_) => anyhow::bail!("IPv6 UDP not supported"),
        }
        dgram.extend_from_slice(&target.port().to_be_bytes());
        dgram.extend_from_slice(payload);
        self.sink
            .lock()
            .await
            .send(Message::Binary(Bytes::from(dgram)))
            .await?;
        Ok(())
    }

    /// Returns `(payload_len, source, flow_id)` where `source` is the remote
    /// endpoint that produced the datagram and `flow_id` is the value the
    /// server echoed from our RSV field (0 from legacy servers).
    pub async fn udp_recv(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr, u16)> {
        let mut r = self.source.lock().await;
        loop {
            match r.next().await {
                Some(Ok(Message::Binary(dgram))) => {
                    if dgram.len() < 10 {
                        continue;
                    }
                    let flow_id = u16::from_be_bytes([dgram[0], dgram[1]]);
                    let (source, payload_start) = match dgram[3] {
                        0x01 => {
                            let ip = Ipv4Addr::new(dgram[4], dgram[5], dgram[6], dgram[7]);
                            let port = u16::from_be_bytes([dgram[8], dgram[9]]);
                            (SocketAddr::new(IpAddr::V4(ip), port), 10)
                        }
                        _ => continue,
                    };
                    let payload = &dgram[payload_start..];
                    let n = payload.len().min(buf.len());
                    buf[..n].copy_from_slice(&payload[..n]);
                    return Ok((n, source, flow_id));
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => anyhow::bail!("WebSocket error: {e}"),
                None => anyhow::bail!("WebSocket closed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::accept_async;

    /// Spawns a fake UDP-channel server: reads `{"op":"udp"}`, then for every
    /// datagram echoes a reply frame whose RSV carries the received flow id
    /// and whose source equals the client's target. With `force_zero` the id
    /// is replaced by 0, simulating a legacy server.
    async fn fake_server(port_tx: tokio::sync::oneshot::Sender<u16>, force_zero: bool) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = port_tx.send(port);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(_))) => continue, // {"op":"udp"} handshake
                Some(Ok(Message::Binary(d))) => {
                    if d.len() < 10 {
                        continue;
                    }
                    let mut fid = u16::from_be_bytes([d[0], d[1]]);
                    if force_zero {
                        fid = 0;
                    }
                    let ip = Ipv4Addr::new(d[4], d[5], d[6], d[7]);
                    let p = u16::from_be_bytes([d[8], d[9]]);
                    let mut r = Vec::with_capacity(10 + 4);
                    r.extend_from_slice(&fid.to_be_bytes());
                    r.push(0);
                    r.push(0x01);
                    r.extend_from_slice(&ip.octets());
                    r.extend_from_slice(&p.to_be_bytes());
                    r.extend_from_slice(b"pong");
                    if ws.send(Message::Binary(Bytes::from(r))).await.is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    async fn open_client(port: u16) -> UdpAssociate {
        // The fake server binds its listener synchronously before sending the
        // port, so the first connect always succeeds once the task is polled.
        loop {
            match UdpAssociate::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).await {
                Ok(a) => return a,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        }
    }

    #[tokio::test]
    async fn flow_id_roundtrip() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(fake_server(tx, false));
        let assoc = open_client(rx.await.unwrap()).await;

        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 53);
        assoc.udp_send(target, b"q1", 7).await.unwrap();
        let mut buf = [0u8; 64];
        let (n, src, fid) = assoc.udp_recv(&mut buf).await.unwrap();
        assert_eq!(fid, 7, "flow id must survive the roundtrip");
        assert_eq!(src, target);
        assert_eq!(&buf[..n], b"pong");

        assoc.udp_send(target, b"q2", 9).await.unwrap();
        let (_, _, fid) = assoc.udp_recv(&mut buf).await.unwrap();
        assert_eq!(fid, 9, "second flow keeps its own id");
    }

    #[tokio::test]
    async fn legacy_server_yields_zero_fid() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(fake_server(tx, true));
        let assoc = open_client(rx.await.unwrap()).await;

        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53);
        assoc.udp_send(target, b"q", 42).await.unwrap();
        let (_, _, fid) = assoc.udp_recv(&mut [0u8; 64]).await.unwrap();
        assert_eq!(fid, 0, "legacy server must degrade to the fid=0 path");
    }

    /// Fake CONNECT server: reports the received request, replies `{"ok":true}`
    /// and then echoes every binary frame (a stand-in for the tunneled TCP).
    async fn connect_echo_server(
        port_tx: tokio::sync::oneshot::Sender<u16>,
        req_tx: tokio::sync::oneshot::Sender<String>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = port_tx.send(port);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        let req = match ws.next().await {
            Some(Ok(Message::Text(t))) => t.to_string(),
            other => {
                let _ = req_tx.send(format!("unexpected: {other:?}"));
                return;
            }
        };
        let _ = req_tx.send(req);
        let _ = ws
            .send(Message::Text(Utf8Bytes::from_static(r#"{"ok":true}"#)))
            .await;
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(d))) => {
                    if ws.send(Message::Binary(d)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(_)) => continue,
                _ => break,
            }
        }
    }

    #[tokio::test]
    async fn connect_tcp_sends_request_and_streams_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (ptx, prx) = tokio::sync::oneshot::channel();
        let (rtx, rrx) = tokio::sync::oneshot::channel();
        tokio::spawn(connect_echo_server(ptx, rtx));
        let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), prx.await.unwrap());
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443);

        let mut stream = connect_tcp(proxy, target).await.unwrap();
        let req = rrx.await.unwrap();
        assert!(req.contains(r#""op":"connect""#), "{req}");
        assert!(req.contains(r#""kind":"ip""#), "{req}");
        assert!(req.contains(r#""ip":"8.8.8.8""#), "{req}");
        assert!(req.contains(r#""port":443"#), "{req}");

        stream.write_all(b"TLS-hello").await.unwrap();
        // `write_all` only queues the frame; the writer must flush explicitly
        // (the tunnel's writer task does the same).
        stream.flush().await.unwrap();
        let mut buf = [0u8; 9];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"TLS-hello");
    }

    /// Fake CONNECT server that accepts, replies `{"ok":true}`, deliberately
    /// stalls so the client's kernel send buffer fills up, then drains every
    /// binary frame and reports how many octets arrived in total.
    async fn connect_counting_server(
        port_tx: tokio::sync::oneshot::Sender<u16>,
        total_tx: tokio::sync::oneshot::Sender<usize>,
        stall: std::time::Duration,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = port_tx.send(port);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        match ws.next().await {
            Some(Ok(Message::Text(_))) => {}
            _ => return,
        }
        let _ = ws
            .send(Message::Text(Utf8Bytes::from_static(r#"{"ok":true}"#)))
            .await;
        // Stall before reading: the client's socket buffer fills, so the WS
        // sink must buffer and `poll_write` observes real backpressure.
        tokio::time::sleep(stall).await;
        let mut total = 0usize;
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(d))) => total += d.len(),
                Some(Ok(_)) => continue,
                _ => break,
            }
        }
        let _ = total_tx.send(total);
    }

    /// Regression: under backpressure `poll_write` must report the bytes it
    /// already accepted. Returning `Pending` after `start_send` queued a frame
    /// made `write_all` resubmit the same buffer, duplicating bytes on the wire
    /// and corrupting the tunneled TCP stream (TLS record errors, truncated
    /// downloads).
    #[tokio::test]
    async fn write_all_under_backpressure_delivers_each_byte_exactly_once() {
        use tokio::io::AsyncWriteExt;
        const CHUNK: usize = 64 * 1024;
        const CHUNKS: usize = 64; // 4 MiB total

        let (ptx, prx) = tokio::sync::oneshot::channel();
        let (ttx, trx) = tokio::sync::oneshot::channel();
        tokio::spawn(connect_counting_server(
            ptx,
            ttx,
            std::time::Duration::from_millis(300),
        ));
        let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), prx.await.unwrap());
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443);

        let mut stream = connect_tcp(proxy, target).await.unwrap();

        // Same shape as the tunnel's writer task: write_all + flush per chunk.
        let payload: Vec<u8> = (0..CHUNK).map(|i| (i % 251) as u8).collect();
        for _ in 0..CHUNKS {
            stream.write_all(&payload).await.unwrap();
            stream.flush().await.unwrap();
        }
        drop(stream);

        let total = trx.await.unwrap();
        assert_eq!(
            total,
            CHUNK * CHUNKS,
            "backpressure must not duplicate or drop bytes"
        );
    }

    /// Fake UDP server that hands the received datagram frame to the test and
    /// echoes it back with `pong` appended.
    async fn udp_inspect_server(
        port_tx: tokio::sync::oneshot::Sender<u16>,
        frame_tx: tokio::sync::oneshot::Sender<Vec<u8>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = port_tx.send(port);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(_))) => continue, // {"op":"udp"}
                Some(Ok(Message::Binary(d))) => {
                    let mut r = d.to_vec();
                    r.extend_from_slice(b"pong");
                    let _ = frame_tx.send(d.to_vec());
                    let _ = ws.send(Message::Binary(Bytes::from(r))).await;
                    break;
                }
                _ => break,
            }
        }
    }

    #[tokio::test]
    async fn udp_frame_layout_is_socks5_udp_compatible() {
        let (ptx, prx) = tokio::sync::oneshot::channel();
        let (ftx, frx) = tokio::sync::oneshot::channel();
        tokio::spawn(udp_inspect_server(ptx, ftx));
        let assoc = open_client(prx.await.unwrap()).await;

        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53);
        assoc.udp_send(target, b"dns", 0x1234).await.unwrap();

        let f = frx.await.unwrap();
        assert_eq!(&f[0..2], &0x1234u16.to_be_bytes(), "flow id in RSV");
        assert_eq!(f[2], 0, "FRAG must be 0");
        assert_eq!(f[3], 0x01, "ATYP IPv4");
        assert_eq!(&f[4..8], &[1, 1, 1, 1]);
        assert_eq!(u16::from_be_bytes([f[8], f[9]]), 53);
        assert_eq!(&f[10..], b"dns");

        let mut buf = [0u8; 32];
        let (n, src, fid) = assoc.udp_recv(&mut buf).await.unwrap();
        assert_eq!(fid, 0x1234, "server echoes the flow id");
        assert_eq!(src, target);
        assert_eq!(&buf[..n], b"dnspong");
    }

    /// Fake server that emits two malformed frames before a valid one.
    async fn udp_noisy_server(port_tx: tokio::sync::oneshot::Sender<u16>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = port_tx.send(port);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        // Wait for the {"op":"udp"} handshake.
        if let Some(Ok(Message::Text(_))) = ws.next().await {}
        // 1) shorter than the 10-byte header.
        let _ = ws.send(Message::Binary(Bytes::from_static(&[0, 0, 0, 0, 1]))).await;
        // 2) unsupported ATYP (0x03 = hostname) in the header.
        let mut bad = vec![0, 9, 0, 0x03];
        bad.extend_from_slice(&[0u8; 12]);
        let _ = ws.send(Message::Binary(Bytes::from(bad))).await;
        // 3) valid frame: fid 5, source 9.9.9.9:53, payload "ok".
        let mut good = Vec::new();
        good.extend_from_slice(&5u16.to_be_bytes());
        good.push(0);
        good.push(0x01);
        good.extend_from_slice(&[9, 9, 9, 9]);
        good.extend_from_slice(&53u16.to_be_bytes());
        good.extend_from_slice(b"ok");
        let _ = ws.send(Message::Binary(Bytes::from(good))).await;
        while let Some(Ok(_)) = ws.next().await {}
    }

    #[tokio::test]
    async fn udp_recv_skips_malformed_frames() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(udp_noisy_server(tx));
        let assoc = open_client(rx.await.unwrap()).await;
        let mut buf = [0u8; 32];
        let (n, src, fid) = assoc.udp_recv(&mut buf).await.unwrap();
        assert_eq!(fid, 5);
        assert_eq!(src, SocketAddr::new(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)), 53));
        assert_eq!(&buf[..n], b"ok");
    }

    /// Fake server that replies with a 100-byte payload.
    async fn udp_big_reply_server(port_tx: tokio::sync::oneshot::Sender<u16>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _ = port_tx.send(port);
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(_))) => {
                    let mut r = Vec::new();
                    r.extend_from_slice(&1u16.to_be_bytes());
                    r.push(0);
                    r.push(0x01);
                    r.extend_from_slice(&[9, 9, 9, 9]);
                    r.extend_from_slice(&53u16.to_be_bytes());
                    r.extend_from_slice(&[0xAB; 100]);
                    let _ = ws.send(Message::Binary(Bytes::from(r))).await;
                    break;
                }
                Some(Ok(_)) => continue,
                _ => break,
            }
        }
    }

    #[tokio::test]
    async fn udp_recv_truncates_to_buffer() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(udp_big_reply_server(tx));
        let assoc = open_client(rx.await.unwrap()).await;
        assoc
            .udp_send(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53), b"q", 1)
            .await
            .unwrap();
        let mut buf = [0u8; 8];
        let (n, _, _) = assoc.udp_recv(&mut buf).await.unwrap();
        assert_eq!(n, 8, "payload must be truncated to the caller's buffer");
        assert!(buf.iter().all(|&b| b == 0xAB));
    }
}
