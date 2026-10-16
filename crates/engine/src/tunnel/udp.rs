use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use wintun::Session;

use crate::tunnel::ws::UdpAssociate;

/// Max tracked (flow_id, target) reply routes before oldest-entry eviction.
const FLOW_CAP: usize = 8192;

const IPPROTO_UDP: u8 = 17;

/// Parsed view of one IPv4/UDP packet captured off the tunnel adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Udp4 {
    /// Local application socket the datagram came from.
    pub app: SocketAddr,
    /// Destination the packet was addressed to (the relay target).
    pub target: SocketAddr,
    pub payload: Vec<u8>,
}

/// Parse a raw IPv4/UDP packet. Returns `None` for anything that is not a
/// well-formed IPv4/UDP datagram with a non-empty payload.
///
/// Deliberately a free function (not a method) so the capture decision that
/// backs the "DNS / WebRTC never leak" guarantee can be tested without a live
/// Wintun session or administrator privileges.
pub(crate) fn parse_udp4(raw: &[u8]) -> Option<Udp4> {
    if raw.len() < 20 {
        return None;
    }
    let ihl = (raw[0] & 0x0f) as usize * 4;
    if ihl < 20 || raw.len() < ihl + 8 {
        return None;
    }
    if raw[9] != IPPROTO_UDP {
        return None; // not UDP
    }
    let src_ip = Ipv4Addr::new(raw[12], raw[13], raw[14], raw[15]);
    let dst_ip = Ipv4Addr::new(raw[16], raw[17], raw[18], raw[19]);
    let src_port = u16::from_be_bytes([raw[ihl], raw[ihl + 1]]);
    let dst_port = u16::from_be_bytes([raw[ihl + 2], raw[ihl + 3]]);
    let udp_len = u16::from_be_bytes([raw[ihl + 4], raw[ihl + 5]]) as usize;
    let pstart = ihl + 8;
    let pend = (pstart + udp_len.saturating_sub(8)).min(raw.len());
    if pend <= pstart {
        return None;
    }
    Some(Udp4 {
        app: SocketAddr::V4(SocketAddrV4::new(src_ip, src_port)),
        target: SocketAddr::V4(SocketAddrV4::new(dst_ip, dst_port)),
        payload: raw[pstart..pend].to_vec(),
    })
}

/// Demultiplexing table for the UDP relay.
///
/// Every local app socket gets a unique 16-bit flow id carried in the frame's
/// RSV field; the server binds one upstream socket per id and echoes the id
/// back, so replies are routed via `(flow_id, source)` — two apps may query the
/// same resolver without stealing each other's answers. `flow_id=0` replies
/// come from legacy servers and fall back to the target-keyed map.
#[derive(Debug, Default)]
pub(crate) struct FlowTable {
    /// (flow_id, source) -> local app socket.
    flows: HashMap<(u16, SocketAddr), SocketAddr>,
    /// Eviction order for `flows`.
    ring: VecDeque<(u16, SocketAddr)>,
    /// Local app socket -> assigned flow id.
    ids: HashMap<SocketAddr, u16>,
    /// Legacy fallback for servers that echo flow_id=0: source -> latest app.
    legacy: HashMap<SocketAddr, SocketAddr>,
    next_id: u32,
}

impl FlowTable {
    pub(crate) fn new() -> Self {
        FlowTable {
            next_id: 1,
            ..Default::default()
        }
    }

    /// Stable flow id for `app`, allocating a fresh one on first sight. Id 0 is
    /// reserved for the legacy path, so it is skipped on wraparound.
    fn assign(&mut self, app: SocketAddr) -> u16 {
        if let Some(id) = self.ids.get(&app) {
            return *id;
        }
        let mut n = (self.next_id & 0xffff) as u16;
        self.next_id = self.next_id.wrapping_add(1);
        if n == 0 {
            n = (self.next_id & 0xffff) as u16;
            self.next_id = self.next_id.wrapping_add(1);
        }
        self.ids.insert(app, n);
        n
    }

    /// Reserve/update the reply route for `(flow_id, target)` and keep the
    /// legacy target map current.
    fn remember(&mut self, fid: u16, target: SocketAddr, app: SocketAddr) {
        let key = (fid, target);
        if !self.flows.contains_key(&key) {
            self.flows.insert(key, app);
            self.ring.push_back(key);
            while self.ring.len() > FLOW_CAP {
                if let Some(old) = self.ring.pop_front() {
                    self.flows.remove(&old);
                } else {
                    break;
                }
            }
        }
        // Always maintained so an old server (echoes fid=0) still works.
        self.legacy.insert(target, app);
    }

    /// Assign a flow id and record the route for one outbound datagram.
    pub(crate) fn route_out(&mut self, app: SocketAddr, target: SocketAddr) -> u16 {
        let fid = self.assign(app);
        self.remember(fid, target, app);
        fid
    }

    /// Resolve the local app socket for an inbound reply.
    pub(crate) fn route_in(&self, fid: u16, source: SocketAddr) -> Option<SocketAddr> {
        if fid != 0 {
            self.flows.get(&(fid, source)).copied()
        } else {
            self.legacy.get(&source).copied()
        }
    }

    #[cfg(test)]
    fn flow_count(&self) -> usize {
        self.flows.len()
    }
}

/// UDP relay: app UDP packets are parsed at the raw IP layer, sent to the
/// device proxy over the WebSocket UDP channel, and replies are rebuilt as
/// IPv4/UDP packets and written straight back to Wintun.
///
/// NOTE: smoltcp's `UdpSocket::recv_from` only exposes the *source* of a
/// datagram, not its destination, so we cannot use smoltcp's UDP socket for
/// tunneling (we must know the destination to relay it). The destination is
/// recovered from the original packet here instead.
#[derive(Clone)]
pub struct UdpRelay {
    to_proxy: mpsc::Sender<(SocketAddr, Vec<u8>, u16)>,
    table: Arc<std::sync::Mutex<FlowTable>>,
    stop_tx: tokio::sync::watch::Sender<bool>,
}

impl UdpRelay {
    pub fn new(assoc: Arc<UdpAssociate>, session: Arc<Session>, rt: &Handle) -> Self {
        let (tx, mut rx) = mpsc::channel::<(SocketAddr, Vec<u8>, u16)>(2048);
        let table = Arc::new(std::sync::Mutex::new(FlowTable::new()));
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

        // Task: forward app datagrams to the proxy.
        let a2 = assoc.clone();
        let mut stop_rx_up = stop_rx.clone();
        rt.spawn(async move {
            let mut up_count: u64 = 0;
            let mut up_bytes: u64 = 0;
            loop {
                tokio::select! {
                    _ = stop_rx_up.changed() => break,
                    msg = rx.recv() => {
                        match msg {
                            Some((target, payload, fid)) => match a2.udp_send(target, &payload, fid).await {
                                Ok(()) => {
                                    up_count += 1;
                                    up_bytes += payload.len() as u64;
                                    crate::add_uplink(payload.len() as u64);
                                }
                                Err(e) => {
                                    crate::log_warn(format!("UDP send error ({}): {}", target, e));
                                }
                            },
                            None => break,
                        }
                    }
                }
            }
            crate::log_msg(format!("UDP up task finished ({} pkts, {} bytes)", up_count, up_bytes));
        });

        // Task: read replies from the proxy and inject them back to Wintun.
        let t2 = table.clone();
        let s2 = session.clone();
        rt.spawn(async move {
            let mut stop_rx = stop_rx;
            let mut buf = [0u8; 65536];
            let mut down_count: u64 = 0;
            let mut down_bytes: u64 = 0;
            loop {
                let r;
                tokio::select! {
                    _ = stop_rx.changed() => break,
                    rr = assoc.udp_recv(&mut buf) => r = rr,
                }
                match r {
                    Ok((n, source, fid)) => {
                        let app = t2.lock().unwrap().route_in(fid, source);
                        if let Some(app) = app {
                            let packet = build_udp4(
                                match source {
                                    SocketAddr::V4(v4) => v4,
                                    _ => continue,
                                },
                                match app {
                                    SocketAddr::V4(v4) => v4,
                                    _ => continue,
                                },
                                &buf[..n],
                            );
                            if let Ok(mut p) = s2.allocate_send_packet(packet.len() as u16) {
                                p.bytes_mut().copy_from_slice(&packet);
                                let _ = s2.send_packet(p);
                                down_count += 1;
                                down_bytes += n as u64;
                                crate::add_downlink(n as u64);
                            }
                        } else {
                            crate::log_warn(format!("no matching client for UDP reply: {} (fid {})",
                                source, fid));
                        }
                    }
                    Err(e) => {
                        crate::log_warn(format!("UDP reply read error: {}", e));
                        break;
                    }
                }
            }
            crate::log_msg(format!("UDP down task finished ({} pkts, {} bytes)", down_count, down_bytes));
        });

        Self { to_proxy: tx, table, stop_tx }
    }

    /// Signal the background tasks to stop. They will release the session
    /// Arc clones they hold, allowing the Wintun session to end.
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Handle one raw IPv4/UDP packet coming from the OS.
    pub fn handle_incoming(&self, raw: &[u8]) {
        let Some(pkt) = parse_udp4(raw) else {
            return;
        };
        let fid = self.table.lock().unwrap().route_out(pkt.app, pkt.target);
        if self.to_proxy.try_send((pkt.target, pkt.payload, fid)).is_err() {
            crate::log_warn(format!("UDP queue full, dropped packet: {} -> {}", pkt.app, pkt.target));
        }
    }
}

/// Build a minimal IPv4 + UDP packet (UDP checksum left 0, which is legal for
/// IPv4). `src` is the real target, `dst` is the local app.
fn build_udp4(src: SocketAddrV4, dst: SocketAddrV4, payload: &[u8]) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let total = 20 + udp_len;
    let mut pkt = vec![0u8; total];

    pkt[0] = 0x45;
    pkt[1] = 0;
    pkt[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    let id = rand::random::<u16>();
    pkt[4..6].copy_from_slice(&id.to_be_bytes());
    pkt[6..8].copy_from_slice(&0u16.to_be_bytes());
    pkt[8] = 64;
    pkt[9] = 17; // UDP
    pkt[12..16].copy_from_slice(&src.ip().octets());
    pkt[16..20].copy_from_slice(&dst.ip().octets());
    let sum = ipv4_checksum(&pkt[..20]);
    pkt[10..12].copy_from_slice(&sum.to_be_bytes());

    let off = 20;
    pkt[off..off + 2].copy_from_slice(&src.port().to_be_bytes());
    pkt[off + 2..off + 4].copy_from_slice(&dst.port().to_be_bytes());
    pkt[off + 4..off + 6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    pkt[off + 6..off + 8].copy_from_slice(&0u16.to_be_bytes());
    pkt[off + 8..].copy_from_slice(payload);
    pkt
}

fn ipv4_checksum(hdr: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < hdr.len() {
        let w = u16::from_be_bytes([hdr[i], hdr[i + 1]]);
        sum += w as u32;
        i += 2;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !sum as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(ip: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
            port,
        ))
    }

    /// Raw IPv4/UDP packet produced by the relay's own builder (IHL = 5).
    fn raw_query(src: SocketAddr, dst: SocketAddr, payload: &[u8]) -> Vec<u8> {
        let (SocketAddr::V4(s), SocketAddr::V4(d)) = (src, dst) else {
            panic!("IPv4 only");
        };
        build_udp4(s, d, payload)
    }

    /// Raw IPv4/UDP packet with IP options (IHL = 6 => 24-byte header).
    fn raw_query_with_options(src: SocketAddr, dst: SocketAddr, payload: &[u8]) -> Vec<u8> {
        let (SocketAddr::V4(s), SocketAddr::V4(d)) = (src, dst) else {
            panic!("IPv4 only");
        };
        let off = 24;
        let mut pkt = vec![0u8; off + 8 + payload.len()];
        pkt[0] = 0x46; // version 4, IHL = 6 words
        let total = pkt.len() as u16;
        pkt[2..4].copy_from_slice(&total.to_be_bytes());
        pkt[9] = IPPROTO_UDP;
        pkt[12..16].copy_from_slice(&s.ip().octets());
        pkt[16..20].copy_from_slice(&d.ip().octets());
        pkt[off..off + 2].copy_from_slice(&s.port().to_be_bytes());
        pkt[off + 2..off + 4].copy_from_slice(&d.port().to_be_bytes());
        pkt[off + 4..off + 6].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        pkt[off + 8..].copy_from_slice(payload);
        pkt
    }

    // ── packet parsing: the "is it captured by the tunnel?" decision ──────

    #[test]
    fn parse_roundtrips_built_packet() {
        let app = addr([10, 8, 0, 2], 40001);
        let target = addr([1, 1, 1, 1], 53);
        let payload = b"\x12\x34dns-query";
        let raw = raw_query(app, target, payload);
        let p = parse_udp4(&raw).expect("valid UDP must parse");
        assert_eq!(p.app, app);
        assert_eq!(p.target, target);
        assert_eq!(p.payload, payload);
    }

    #[test]
    fn parse_honours_ip_options() {
        let app = addr([10, 8, 0, 2], 50000);
        let target = addr([9, 9, 9, 9], 123);
        let raw = raw_query_with_options(app, target, b"opt");
        let p = parse_udp4(&raw).expect("IHL=6 packet must parse");
        assert_eq!(p.app, app);
        assert_eq!(p.target, target);
        assert_eq!(p.payload, b"opt");
    }

    #[test]
    fn parse_rejects_non_udp() {
        let mut raw = raw_query(addr([10, 8, 0, 2], 1), addr([1, 1, 1, 1], 80), b"x");
        raw[9] = 6; // TCP
        assert!(parse_udp4(&raw).is_none());
    }

    #[test]
    fn parse_rejects_truncated_and_empty() {
        assert!(parse_udp4(&[0u8; 19]).is_none(), "short packet");
        // IPv4 header only, no UDP header.
        let mut raw = raw_query(addr([10, 8, 0, 2], 1), addr([1, 1, 1, 1], 80), b"x");
        raw.truncate(20);
        assert!(parse_udp4(&raw).is_none(), "missing UDP header");
        // UDP length 8 => zero-length payload is dropped (DNS is never empty).
        let mut raw = raw_query(addr([10, 8, 0, 2], 1), addr([1, 1, 1, 1], 80), b"x");
        raw[24..26].copy_from_slice(&8u16.to_be_bytes());
        assert!(parse_udp4(&raw).is_none(), "empty payload");
    }

    /// DNS: a UDP/53 query to the pinned resolver must be captured with its
    /// destination intact — nothing about it may bypass the tunnel.
    #[test]
    fn dns_query_is_captured_with_resolver_intact() {
        let app = addr([10, 8, 0, 2], 52345);
        let query = [0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
        for resolver in [[1, 1, 1, 1], [8, 8, 8, 8]] {
            let raw = raw_query(app, addr(resolver, 53), &query);
            let p = parse_udp4(&raw).expect("DNS query must be captured");
            assert_eq!(p.target, addr(resolver, 53));
            assert_eq!(p.payload, query);
        }
    }

    /// WebRTC STUN/TURN: arbitrary UDP to a public server (no fixed port) is
    /// captured and forwarded — a classic WebRTC leak vector stays in-tunnel.
    #[test]
    fn webrtc_stun_is_captured() {
        let app = addr([10, 8, 0, 2], 51413);
        // STUN Binding Request: type 0x0001, magic cookie 0x2112A442.
        let stun = [
            0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
        ];
        let raw = raw_query(app, addr([74, 125, 228, 1], 19302), &stun);
        let p = parse_udp4(&raw).expect("STUN must be captured");
        assert_eq!(p.target, addr([74, 125, 228, 1], 19302));
        assert_eq!(p.payload, stun);
    }

    /// mDNS (224.0.0.251:5353) must be captured too: WebRTC host candidates
    /// must not reach the physical LAN directly.
    #[test]
    fn mdns_multicast_is_captured() {
        let app = addr([10, 8, 0, 2], 5353);
        let raw = raw_query(app, addr([224, 0, 0, 251], 5353), b"mdns");
        let p = parse_udp4(&raw).expect("mDNS must be captured");
        assert_eq!(p.target, addr([224, 0, 0, 251], 5353));
        assert_eq!(p.payload, b"mdns");
    }

    // ── reply construction ────────────────────────────────────────────────

    #[test]
    fn build_udp4_fields_and_checksum() {
        let src = SocketAddrV4::new(Ipv4Addr::new(8, 8, 8, 8), 53);
        let dst = SocketAddrV4::new(Ipv4Addr::new(10, 8, 0, 2), 52345);
        let pkt = build_udp4(src, dst, b"answer");
        assert_eq!(pkt[0], 0x45, "version 4 / IHL 5");
        assert_eq!(pkt[9], 17, "protocol UDP");
        assert_eq!(
            u16::from_be_bytes([pkt[2], pkt[3]]) as usize,
            pkt.len(),
            "IPv4 total length"
        );
        assert_eq!(&pkt[12..16], &[8, 8, 8, 8]);
        assert_eq!(&pkt[16..20], &[10, 8, 0, 2]);
        assert_eq!(u16::from_be_bytes([pkt[20], pkt[21]]), 53);
        assert_eq!(u16::from_be_bytes([pkt[22], pkt[23]]), 52345);
        assert_eq!(
            u16::from_be_bytes([pkt[24], pkt[25]]) as usize,
            8 + 6,
            "UDP length"
        );
        assert_eq!(&pkt[28..], b"answer");
        assert_eq!(ipv4_checksum(&pkt[..20]), 0, "header checksum must validate");
    }

    #[test]
    fn build_udp4_roundtrips_through_parse() {
        let app = addr([10, 8, 0, 2], 40000);
        let target = addr([1, 1, 1, 1], 53);
        let raw = raw_query(app, target, b"roundtrip");
        let p = parse_udp4(&raw).unwrap();
        assert_eq!(p.app, app);
        assert_eq!(p.target, target);
        assert_eq!(p.payload, b"roundtrip");
    }

    // ── flow demux: the anti-leak / anti-hijack core ──────────────────────

    #[test]
    fn flow_table_ids_are_unique_and_stable() {
        let mut t = FlowTable::new();
        let a = addr([10, 8, 0, 2], 40001);
        let b = addr([10, 8, 0, 2], 40002);
        let id_a1 = t.route_out(a, addr([1, 1, 1, 1], 53));
        let id_b = t.route_out(b, addr([1, 1, 1, 1], 53));
        let id_a2 = t.route_out(a, addr([8, 8, 8, 8], 53));
        assert_ne!(id_a1, 0);
        assert_ne!(id_b, 0);
        assert_ne!(id_a1, id_b, "distinct apps get distinct ids");
        assert_eq!(id_a1, id_a2, "same app keeps its id across targets");
    }

    /// Two apps querying the *same* resolver must receive their own answers.
    #[test]
    fn flow_table_shared_resolver_no_crosstalk() {
        let mut t = FlowTable::new();
        let resolver = addr([1, 1, 1, 1], 53);
        let a = addr([10, 8, 0, 2], 40001);
        let b = addr([10, 8, 0, 2], 40002);
        let ia = t.route_out(a, resolver);
        let ib = t.route_out(b, resolver);

        assert_eq!(t.route_in(ia, resolver), Some(a), "A's answer -> A");
        assert_eq!(t.route_in(ib, resolver), Some(b), "B's answer -> B");
    }

    #[test]
    fn flow_table_legacy_zero_fid_falls_back_to_source() {
        let mut t = FlowTable::new();
        let resolver = addr([8, 8, 8, 8], 53);
        let a = addr([10, 8, 0, 2], 40001);
        t.route_out(a, resolver);
        // Legacy server echoes fid=0; route by source only.
        assert_eq!(t.route_in(0, resolver), Some(a));
        assert_eq!(t.route_in(0, addr([9, 9, 9, 9], 53)), None);
    }

    #[test]
    fn flow_table_skips_zero_id_on_wraparound() {
        let mut t = FlowTable::new();
        t.next_id = 0xffff; // next allocation would be 65535, then wrap to 0
        let a = addr([10, 8, 0, 2], 40001);
        let b = addr([10, 8, 0, 2], 40002);
        let first = t.route_out(a, addr([1, 1, 1, 1], 53));
        let second = t.route_out(b, addr([1, 1, 1, 1], 53));
        assert_eq!(first, 0xffff);
        assert_eq!(second, 1, "id 0 is reserved and skipped");
    }

    #[test]
    fn flow_table_evicts_oldest_at_cap() {
        let mut t = FlowTable::new();
        let target = addr([1, 1, 1, 1], 53);
        let app = addr([10, 8, 0, 2], 40000);
        // Fill one past the cap with distinct flow ids.
        for i in 1..=(FLOW_CAP as u16 + 8) {
            t.remember(i, target, app);
        }
        assert!(
            t.flow_count() <= FLOW_CAP,
            "table must not exceed the cap: {}",
            t.flow_count()
        );
        assert_eq!(t.route_in(1, target), None, "oldest entry evicted");
        let last = FLOW_CAP as u16 + 8;
        assert_eq!(t.route_in(last, target), Some(app), "newest entry kept");
    }
}
