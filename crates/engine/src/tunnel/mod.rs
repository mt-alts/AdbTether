pub mod device;
pub mod udp;
pub mod ws;

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration as StdDuration;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer as TcpSocketBuffer, State};
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address, Ipv4Cidr, IpEndpoint};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, split};
use wintun::{Adapter, Session};

use device::WintunDevice;
use ws::{UdpAssociate, connect_tcp};
use udp::UdpRelay;const ADAPTER_NAME: &str = "AdbTether";
const TUNNEL_IP: &str = "10.8.0.2";
const GATEWAY: &str = "10.8.0.1";
const RING_CAP: u32 = wintun::MAX_RING_CAPACITY;

/// Uplink chunks buffered between the poll loop and the bridge task. Once the
/// backlog reaches this many chunks we stop draining smoltcp's rx ring, which
/// shrinks the window we advertise to the app (backpressure, not loss).
const APP_PENDING_CHUNKS: usize = 32;
/// Downlink chunks buffered between the bridge task and smoltcp. Once full we
/// stop draining the channel, which blocks the bridge reader and pushes the
/// backpressure out to the phone.
const NET_PENDING_CHUNKS: usize = 32;
/// Capacity of the two per-connection bridge channels, in chunks. Kept small
/// on purpose: a deep queue only adds latency, and 8192 x 64 KiB let a single
/// connection buffer hundreds of megabytes.
const BRIDGE_CHANNEL: usize = 64;

/// Hand queued downlink chunks to `write`, which reports how many octets it
/// actually accepted — a smoltcp tx ring that is nearly full takes only part
/// of a chunk (`send_slice` enqueues the largest contiguous free slice).
///
/// Whatever the sink refuses stays at the front of `pending`; `off` is how far
/// into that front chunk we have already written. No byte is ever discarded,
/// which is what the old `min(chunk.len(), written)` accounting got wrong: it
/// dropped the remainder of every burst larger than the ring.
fn drain_pending<F>(pending: &mut VecDeque<Vec<u8>>, off: &mut usize, mut write: F)
where
    F: FnMut(&[u8]) -> usize,
{
    loop {
        let n = match pending.front() {
            Some(front) => write(&front[*off..]),
            None => return,
        };
        if n == 0 {
            return; // sink full — retry on the next poll
        }
        *off += n;
        match pending.front() {
            Some(front) if *off == front.len() => {
                pending.pop_front();
                *off = 0;
            }
            _ => return, // sink full again
        }
    }
}

pub struct Tunnel {
    adapter: Option<Arc<Adapter>>,
    session: Mutex<Option<Arc<Session>>>,
    running: Arc<Mutex<bool>>,
    pub udp_supported: bool,
    device: String,
    pub filter_hosts: String,
    pub proxy: SocketAddr,
    pub suspended: bool,
    /// Worker threads spawned by `up`; `down` joins them so the session's
    /// Arc clones are dropped before the session itself is taken.
    workers: Vec<thread::JoinHandle<()>>,
    /// UDP relay (holds the proxy WebSocket + background tasks). Stopped in
    /// `down` so its session Arc clones are released.
    udp_relay: Option<UdpRelay>,
    /// System DNS servers captured at `up` time together with the physical
    /// gateway/interface that reached them BEFORE our own default route
    /// shadows the table: (ip, gateway, if_index).
    dns_routes: Vec<(std::net::Ipv4Addr, String, String)>,
    /// True while DNS-bypass /32 routes are installed (DNS_MODE=1).
    dns_bypass_active: bool,
    /// True while the ::/0 IPv6 blackhole route is installed.
    v6_blackhole_active: bool,
}

/// Bridge state for one smoltcp TCP connection.
struct TcpConn {
    /// poll loop -> bridge task (data from the app, destined for the internet).
    app_to_net_tx: mpsc::Sender<Vec<u8>>,
    /// bridge task -> poll loop (data from the internet, destined for the app).
    net_to_app_rx: mpsc::Receiver<Vec<u8>>,
    /// Backpressure buffer when `app_to_net_tx` is full.
    pending: VecDeque<Vec<u8>>,
    /// Downlink chunks the smoltcp tx ring could not take yet. `send_slice`
    /// accepts only what fits, so leftovers wait here for the next poll rather
    /// than being dropped (dropping them silently corrupts the app's stream).
    net_pending: VecDeque<Vec<u8>>,
    /// Octets of `net_pending.front()` already enqueued into smoltcp.
    net_pending_off: usize,
}

fn ep_to_sock(ep: IpEndpoint) -> SocketAddr {
    let IpAddress::Ipv4(v4) = ep.addr;
    SocketAddr::new(IpAddr::V4(Ipv4Addr::from(v4.octets())), ep.port)
}

#[allow(dead_code)]
fn sock_to_ep(sa: SocketAddr) -> IpEndpoint {
    match sa {
        SocketAddr::V4(v4) => IpEndpoint::new(
            IpAddress::from(Ipv4Address::new(
                v4.ip().octets()[0],
                v4.ip().octets()[1],
                v4.ip().octets()[2],
                v4.ip().octets()[3],
            )),
            v4.port(),
        ),
        _ => IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), sa.port()),
    }
}

impl Tunnel {
    /// First-time setup: load wintun, open or create the adapter.
    /// Does NOT create a session or start traffic. Call `up` afterwards.
    pub fn open(device_id: &str) -> anyhow::Result<Self> {
        crate::log_msg(format!("loading wintun.dll..."));
        let wintun = unsafe { wintun::load_from_path("wintun.dll") }
            .map_err(|e| anyhow::anyhow!("failed to load wintun.dll: {e}"))?;
        crate::log_msg(format!("wintun.dll loaded"));

        crate::log_msg(format!("preparing TUN adapter: {}", ADAPTER_NAME));
        let adapter = Adapter::open(&wintun, ADAPTER_NAME)
            .or_else(|_| Adapter::create(&wintun, ADAPTER_NAME, "AdbTether Tunnel", None))
            .map_err(|e| {
                anyhow::anyhow!(
                    "could not create TUN adapter (admin/administrator privileges may be required): {e}"
                )
            })?;

        Ok(Self {
            adapter: Some(adapter),
            session: Mutex::new(None),
            running: Arc::new(Mutex::new(true)),
            udp_supported: false,
            device: device_id.to_string(),
            filter_hosts: String::new(),
            proxy: "127.0.0.1:0".parse().unwrap(),
            suspended: false,
            workers: Vec::new(),
            udp_relay: None,
            dns_routes: Vec::new(),
            dns_bypass_active: false,
            v6_blackhole_active: false,
        })
    }

    /// Bring the tunnel up: install routes, create a session, start smoltcp
    /// and the reader/poll threads. Safe to call again after `down` (the
    /// adapter is reused, so reconnect is fast).
    pub async fn up(&mut self, proxy: SocketAddr, filter_hosts: &str, rt: Handle) -> anyhow::Result<()> {
        self.proxy = proxy;
        self.filter_hosts = filter_hosts.to_string();
        crate::log_msg(format!("Tunnel starting: proxy={}", proxy));
        crate::log_msg(format!("wintun adapter ready: {}", ADAPTER_NAME));

        set_ip(TUNNEL_IP)?;
        // Capture system DNS servers + their physical path BEFORE our own
        // default route shadows the routing table; needed for DNS_MODE=1.
        self.capture_dns_servers();
        if crate::dns_mode() == 1 {
            self.install_dns_bypass();
        }
        add_device_exclusion(&self.device)?;
        add_host_exclusions(filter_hosts)?;
        add_default_route()?;

        // IPv6 kill-switch (user-configurable): the data path is v4-only, so
        // blackhole ::/0 to stop AAAA/DNS/WebRTC leaks via the physical
        // adapter. Non-fatal on systems where v6 is disabled.
        if crate::ipv6_block_enabled() {
            self.install_v6_blackhole();
        } else {
            crate::log_msg("IPv6 kill-switch disabled by setting".into());
        }

        let adapter = self.adapter.as_ref().expect("adapter must exist").clone();
        let session = Arc::new(adapter.start_session(RING_CAP)?);
        *self.session.lock().unwrap() = Some(session.clone());
        *self.running.lock().unwrap() = true;

        // ── smoltcp network stack ──────────────────────────────────────
        let (mut device, rx_queue) = WintunDevice::new(session.clone());

        let mut iface = Interface::new(
            Config::new(smoltcp::wire::HardwareAddress::Ip),
            &mut device,
            Instant::now(),
        );
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(Ipv4Address::new(10, 8, 0, 2), 24)));
        });
        iface.set_any_ip(true);
        // Default route: gateway = interface's own IP (10.8.0.2) → AnyIP
        // so packets to all destinations are accepted locally
        let old = iface.routes_mut().add_default_ipv4_route(Ipv4Address::new(10, 8, 0, 2));
        crate::log_msg(format!("smoltcp default route -> 10.8.0.2 (AnyIP, old={:?})", old));
        crate::log_msg(format!("smoltcp interface ready (10.8.0.2/24, AnyIP, default route)"));

        let mut sockets = SocketSet::new(vec![]);

        // Reader → poll loop channel: TCP SYN packets (port + raw bytes).
        // SYNs go to this channel (not rx_queue); the poll loop creates the
        // listener, then pushes them to rx_queue.
        let (syn_tx, mut syn_rx) = mpsc::channel::<(u16, Vec<u8>)>(1024);

        // ── UDP relay (raw IP layer) ──────────────────────────────────
        let (udp_ok, relay) = match UdpAssociate::bind(self.proxy).await {
            Ok(assoc) => {
                crate::log_msg(format!("UDP assoc bound (tunnel over TCP): proxy={}", self.proxy));
                let a = Arc::new(assoc);
                let r = UdpRelay::new(a.clone(), session.clone(), &rt);
                (true, Some(r))
            }
            Err(e) => {
                crate::log_msg(format!("WARNING: UDP assoc failed ({}). Tunnel will run TCP-only; DNS/UDP \
                     traffic may not flow.",
                    e));
                (false, None)
            }
        };
        self.udp_supported = udp_ok;

        // Pin adapter DNS only when UDP actually flows. In TCP-only fallback
        // the pinned resolvers would be unreachable (UDP is blackholed), so
        // we leave the adapter's DNS untouched and physical resolution keeps
        // working — a deliberate fail-open for availability.
        if udp_ok {
            if let Err(e) = apply_adapter_dns() {
                crate::log_warn(format!("adapter DNS pin failed: {e}"));
            }
        } else {
            crate::log_warn(
                "TCP-only fallback: adapter DNS left untouched, physical DNS stays active".into(),
            );
        }

        // Reader thread: TCP -> smoltcp rx queue, UDP -> relay (raw layer).
        {
            let relay = relay.clone();
            let rx = rx_queue.clone();
            let s = session.clone();
            let syn_tx = syn_tx;
            self.workers.push(thread::spawn(move || {
                crate::log_msg(format!("Wintun reader thread started"));
                let mut tcp_count: u64 = 0;
                let mut udp_count: u64 = 0;
                let mut other_count: u64 = 0;
                loop {
                    match s.receive_blocking() {
                        Ok(pkt) => {
                            let bytes = pkt.bytes();
                            match ipv4_proto(bytes) {
                                Some(IPPROTO_UDP) => {
                                    udp_count += 1;
                                    if let Some(r) = &relay {
                                        r.handle_incoming(bytes);
                                    }
                                }
                                Some(IPPROTO_TCP) => {
                                    tcp_count += 1;
                                    // TCP SYN? → buffer in channel (until listener is ready)
                                    if is_tcp_syn(bytes) {
                                        let ihl = (bytes[0] & 0x0f) as usize * 4;
                                        let dst_port = u16::from_be_bytes([bytes[ihl + 2], bytes[ihl + 3]]);
                                        if syn_tx.try_send((dst_port, bytes.to_vec())).is_err() {
                                            crate::log_warn(format!("SYN channel full, dropped port {}", dst_port));
                                        }
                                    } else {
                                        // Non-SYN TCP packets go directly to rx_queue
                                        rx.lock().unwrap().push_back(bytes.to_vec());
                                    }
                                }
                                // IPv6 or anything unrecognised is handed to smoltcp,
                                // which is IPv4-only and drops it. IPv6 never escapes
                                // via the adapter (the ::/0 blackhole handles the OS).
                                _ => {
                                    other_count += 1;
                                    rx.lock().unwrap().push_back(bytes.to_vec());
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                crate::log_msg(format!("Wintun reader thread stopped (TCP={}, UDP={}, other={})", tcp_count, udp_count, other_count));
            }));
        }

        // (dst_ip[4], dst_port) → SocketHandle (aktif dinleyiciler)
        let listener_map: Arc<Mutex<HashMap<([u8; 4], u16), SocketHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let conn_state: Arc<Mutex<HashMap<SocketHandle, TcpConn>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let poll_running = self.running.clone();
        let poll_proxy = self.proxy;
        let poll_rt = rt.clone();

        self.workers.push(thread::spawn(move || {
            crate::log_msg(format!("smoltcp poll loop started"));
            loop {
                if !*poll_running.lock().unwrap() {
                    break;
                }
                let now = Instant::now();

                // 1) Process SYNs: create listener, then push to rx_queue
                //    (BEFORE iface.poll — the listener must be ready)
                while let Ok((dst_port, pkt)) = syn_rx.try_recv() {
                    // Extract destination IP from the SYN packet
                    if pkt.len() < 20 {
                        rx_queue.lock().unwrap().push_back(pkt);
                        continue;
                    }
                    let dst_ip: [u8; 4] = [pkt[16], pkt[17], pkt[18], pkt[19]];
                    let key = (dst_ip, dst_port);
                    let dst_ip_addr = Ipv4Address::new(dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]);

                    let mut lm = listener_map.lock().unwrap();
                    if !lm.contains_key(&key) {
                        let mut sock = TcpSocket::new(
                            TcpSocketBuffer::new(vec![0u8; 1048576]),
                            TcpSocketBuffer::new(vec![0u8; 1048576]),
                        );
                        let ep = IpEndpoint::new(
                            IpAddress::Ipv4(dst_ip_addr),
                            dst_port,
                        );
                        if sock.listen(ep).is_ok() {
                            let h = sockets.add(sock);
                            lm.insert(key, h);
                        } else {
                            crate::log_warn(format!("TCP listener {}:{} FAILED TO ADD", dst_ip_addr, dst_port));
                        }
                    }
                    drop(lm);
                    // Listener ready → give the SYN to smoltcp
                    rx_queue.lock().unwrap().push_back(pkt);
                }

                // 2) smoltcp: match SYNs against listener sockets
                let _ = iface.poll(now, &mut device, &mut sockets);

                // Detect accepted connections
                let accepted: Vec<(SocketHandle, SocketAddr)> = {
                    let mut lm = listener_map.lock().unwrap();
                    let mut acc = Vec::new();
                    let mut to_remove = Vec::new();
                    for (&key, &h) in lm.iter() {
                        let s = sockets.get_mut::<TcpSocket>(h);
                        if s.is_open() && s.state() == State::Established {
                            let target = ep_to_sock(s.local_endpoint().unwrap());
                            acc.push((h, target));
                            to_remove.push(key);
                        }
                    }
                    for key in &to_remove {
                        lm.remove(key);
                    }
                    acc
                };

                for (h, target) in accepted {
                    // Create new listener (so the port listens again)
                    let dst_ip = match target.ip() {
                        IpAddr::V4(v4) => v4.octets(),
                        _ => [0u8; 4],
                    };
                    let dst_port = target.port();
                    let key = (dst_ip, dst_port);
                    {
                        let mut lm = listener_map.lock().unwrap();
                        if !lm.contains_key(&key) {
                            let mut sock = TcpSocket::new(
                                TcpSocketBuffer::new(vec![0u8; 1048576]),
                                TcpSocketBuffer::new(vec![0u8; 1048576]),
                            );
                            let ip_addr = Ipv4Address::new(dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3]);
                            let ep = IpEndpoint::new(
                                IpAddress::Ipv4(ip_addr),
                                dst_port,
                            );
                            if sock.listen(ep).is_ok() {
                                let h2 = sockets.add(sock);
                                lm.insert(key, h2);
                            }
                        }
                    }

                    let (a2n_tx, a2n_rx) = mpsc::channel::<Vec<u8>>(BRIDGE_CHANNEL);
                    let (n2a_tx, n2a_rx) = mpsc::channel::<Vec<u8>>(BRIDGE_CHANNEL);
                    conn_state.lock().unwrap().insert(
                        h,
                        TcpConn {
                            app_to_net_tx: a2n_tx,
                            net_to_app_rx: n2a_rx,
                            pending: VecDeque::new(),
                            net_pending: VecDeque::new(),
                            net_pending_off: 0,
                        },
                    );

                    let proxy = poll_proxy;
                    let rt2 = poll_rt.clone();
                    rt2.spawn(async move {
                        match connect_tcp(proxy, target).await {
                            Ok(stream) => {
                                let (mut r, mut w) = split(stream);
                                let writer = {
                                    let mut rx = a2n_rx;
                                    tokio::spawn(async move {
                                        let mut total: u64 = 0;
                                        while let Some(data) = rx.recv().await {
                                            total += data.len() as u64;
                                            crate::add_uplink(data.len() as u64);
                                            if w.write_all(&data).await.is_err() {
                                                break;
                                            }
                                            // The WS sink buffers the frame; flush it
                                            // so it actually reaches the socket, and
                                            // so congestion is observed here.
                                            if w.flush().await.is_err() {
                                                break;
                                            }
                                        }
                                        total
                                    })
                                };
                                let reader = {
                                    let tx = n2a_tx;
                                    tokio::spawn(async move {
                                        let mut buf = [0u8; 65536];
                                        let mut total: u64 = 0;
                                        loop {
                                            match r.read(&mut buf).await {
                                                Ok(0) | Err(_) => break,
                                                Ok(n) => {
                                                    total += n as u64;
                                                    crate::add_downlink(n as u64);
                                                    if tx.send(buf[..n].to_vec()).await.is_err() {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        total
                                    })
                                };
                                let _ = tokio::join!(writer, reader);
                            }
                            Err(e) => {
                                crate::log_warn(format!("CONNECT error {}: {}", target, e));
                            }
                        }
                    });
                }

                // Transfer data for established connections.
                let closed: Vec<SocketHandle> = {
                    let mut cs = conn_state.lock().unwrap();
                    let mut to_close = Vec::new();
                    for (h, st) in cs.iter_mut() {
                        let s = sockets.get_mut::<TcpSocket>(*h);
                        let may_recv = s.may_recv();
                        let may_send = s.may_send();
                        // 1) Flush the app->net backlog into the bridge channel.
                        while let Some(data) = st.pending.pop_front() {
                            match st.app_to_net_tx.try_send(data) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(d)) => {
                                    st.pending.push_front(d);
                                    break;
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => break,
                            }
                        }
                        // 2) Move app bytes out of the smoltcp rx ring, but stop
                        //    once the backlog is full. The bytes then stay in
                        //    smoltcp's buffer, which shrinks the window we
                        //    advertise and slows the app down — backpressure,
                        //    never loss.
                        if may_recv && st.pending.len() < APP_PENDING_CHUNKS {
                            let data: Vec<u8> = s.recv(|b: &mut [u8]| -> (usize, Vec<u8>) {
                                let v = b.to_vec();
                                (v.len(), v)
                            }).unwrap_or_default();
                            if !data.is_empty() {
                                match st.app_to_net_tx.try_send(data) {
                                    Ok(()) => {}
                                    Err(mpsc::error::TrySendError::Full(d)) => {
                                        st.pending.push_back(d);
                                    }
                                    Err(mpsc::error::TrySendError::Closed(_)) => {}
                                }
                            }
                        }
                        // 3) Refill the downlink backlog from the bridge task.
                        if may_send {
                            while st.net_pending.len() < NET_PENDING_CHUNKS {
                                match st.net_to_app_rx.try_recv() {
                                    Ok(d) => st.net_pending.push_back(d),
                                    Err(_) => break,
                                }
                            }
                            // 4) Feed the smoltcp tx ring. `send_slice` enqueues
                            //    only what the ring can actually take and reports
                            //    how much that was; whatever is left stays queued
                            //    for the next poll instead of being discarded.
                            drain_pending(&mut st.net_pending, &mut st.net_pending_off, |b| {
                                s.send_slice(b).unwrap_or(0)
                            });
                        }
                        if !s.is_open() {
                            to_close.push(*h);
                        }
                    }
                    to_close
                };
                if !closed.is_empty() {
                    let mut cs = conn_state.lock().unwrap();
                    for h in closed {
                        cs.remove(&h);
                        sockets.remove(h);
                    }
                }

                // Sleep until smoltcp wants to be polled again.
                // Ama rx_queue'da veri varsa hemen uyan (gecikme yok).
                let has_pending = !rx_queue.lock().unwrap().is_empty();
                let delay = iface.poll_delay(now, &sockets);
                let ms = if has_pending {
                    0
                } else {
                    match delay {
                        Some(d) => d.total_millis().clamp(1, 50),
                        None => 1,
                    }
                };

                // Periyodik durum logu (kaldırıldı — gürültü)

                if ms > 0 {
                    thread::sleep(StdDuration::from_millis(ms));
                }
            }
            crate::log_msg(format!("smoltcp poll loop stopped"));
        }));

        crate::log_msg(format!("Tunnel ready: adapter={}, ip={}/24, udp={}",
            ADAPTER_NAME,
            TUNNEL_IP,
            if self.udp_supported { "supported" } else { "none" }));
        self.udp_relay = relay;
        self.suspended = false;
        Ok(())
    }

    /// Suspend the tunnel: stop the session and remove routes, but KEEP the
    /// adapter so a later `up` reconnects instantly. Safe to call when
    /// already down (no-op).
    pub fn down(&mut self) {
        if self.suspended {
            return;
        }
        crate::log_msg(format!("Tunnel suspending"));
        *self.running.lock().unwrap() = false;
        // Stop the UDP relay first so its background tasks release the
        // session Arc clones they hold.
        if let Some(r) = self.udp_relay.take() {
            r.stop();
        }
        // Give the tokio UDP tasks a moment to observe the stop signal and
        // drop their session Arc clones.
        thread::sleep(StdDuration::from_millis(50));
        // Wake the reader thread so it exits; it holds an Arc clone of the
        // session that must be dropped before the session can end.
        if let Some(s) = self.session.lock().unwrap().as_ref() {
            let _ = s.shutdown();
        }
        // Join workers so all session Arc clones are released.
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
        if let Some(_s) = self.session.lock().unwrap().take() {
            crate::log_msg(format!("Tunnel session ended"));
        }
        let _ = remove_default_route();
        self.remove_v6_blackhole();
        self.remove_dns_bypass();
        clear_adapter_dns();
        remove_host_exclusions(&self.filter_hosts);
        remove_device_exclusion(&self.device);
        self.suspended = true;
        crate::log_msg(format!("Tunnel suspended (adapter kept: {})", ADAPTER_NAME));
    }

    /// Full teardown used when the UI app exits: `down()` plus clearing the
    /// adapter's static IP. Windows is then fully off the tunnel — no routes,
    /// no pinned DNS and no address/subnet on the adapter — while the adapter
    /// itself is kept so the next connect resumes instantly.
    pub fn teardown(&mut self) {
        self.down();
        reset_ip();
    }

    /// Full teardown: suspend then delete the adapter. Used on app shutdown.
    pub fn destroy(mut self) {
        self.down();
        crate::log_msg(format!("Tunnel destroying"));
        if let Some(adapter) = self.adapter.take() {
            if let Ok(a) = Arc::try_unwrap(adapter) {
                let _ = a.delete();
                crate::log_msg(format!("Adapter deleted: {}", ADAPTER_NAME));
            }
        }
        crate::log_msg(format!("Tunnel destroyed"));
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.down();
    }
}

// ── netsh helpers ─────────────────────────────────────────────────

fn netsh(args: &[&str]) -> anyhow::Result<()> {
    crate::log_msg(format!("netsh {}", args.join(" ")));
    let out = Command::new("netsh").args(args).output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let out_s = String::from_utf8_lossy(&out.stdout);
        if !err.is_empty() || out_s.contains("error") {
            let msg = format!("netsh {:?} failed: {} {}", args, out_s, err);
            crate::log_msg(format!("netsh error: {}", msg));
            anyhow::bail!(msg);
        }
    }
    Ok(())
}

fn set_ip(ip: &str) -> anyhow::Result<()> {
    crate::log_msg(format!("Assigning adapter IP: {}/24", ip));
    netsh(&[
        "interface",
        "ip",
        "set",
        "address",
        &format!("name={ADAPTER_NAME}"),
        "static",
        ip,
        "255.255.255.0",
    ])
}

/// Clears the adapter's static address so no tunnel IP/subnet survives a
/// teardown. Best-effort: `up` sets the address again on the next connect.
fn reset_ip() {
    crate::log_msg(format!("Resetting adapter IP: {ADAPTER_NAME} -> DHCP"));
    let _ = netsh(&[
        "interface",
        "ip",
        "set",
        "address",
        &format!("name={ADAPTER_NAME}"),
        "source=dhcp",
    ]);
}

fn add_default_route() -> anyhow::Result<()> {
    crate::log_msg(format!("Adding default route: 0.0.0.0/0 via {} (metric=1)",
        GATEWAY));
    netsh(&[
        "interface",
        "ip",
        "add",
        "route",
        "0.0.0.0/0",
        ADAPTER_NAME,
        GATEWAY,
        "metric=1",
    ])
}

fn remove_default_route() -> anyhow::Result<()> {
    crate::log_msg(format!("Removing default route: 0.0.0.0/0"));
    netsh(&[
        "interface",
        "ip",
        "delete",
        "route",
        "0.0.0.0/0",
        ADAPTER_NAME,
        GATEWAY,
    ])
}

// ――― IPv6 kill-switch ―――――――――――――――――――――――――――――――――――――――――――――――――
// The data path (smoltcp + relays) is IPv4-only, so without countermeasures
// the physical adapter's own ::/0 route keeps sending AAAA traffic, v6 DNS
// and WebRTC STUN straight to the real network. While the tunnel is up we
// anchor a ULA address on the tunnel adapter and point ::/0 at an unroutable
// next-hop there: Windows prefers this route (metric=1) for all v6, the
// packets enter the tunnel and are dropped -> fail-closed, nothing leaks.

const V6_ADDR: &str = "fd8a:8a8a:8a8a::2";
const V6_PREFIX: &str = "64";
const V6_GW: &str = "fd8a:8a8a:8a8a::1";

/// Anchors the ULA address used as the blackhole route's on-link subnet.
fn set_v6_address() -> anyhow::Result<()> {
    crate::log_msg(format!("Assigning adapter IPv6 anchor: {V6_ADDR}/{V6_PREFIX}"));
    netsh(&[
        "interface",
        "ipv6",
        "set",
        "address",
        &format!("interface={ADAPTER_NAME}"),
        &format!("address={V6_ADDR}/{V6_PREFIX}"),
    ])
}

/// Installs the ::/0 blackhole via the unroutable ULA next-hop.
fn add_ipv6_blackhole() -> anyhow::Result<()> {
    set_v6_address()?;
    crate::log_msg("Adding IPv6 kill-switch route: ::/0 via tunnel (blackhole)".into());
    netsh(&[
        "interface",
        "ipv6",
        "add",
        "route",
        "::/0",
        ADAPTER_NAME,
        V6_GW,
        "metric=1",
    ])
}

/// Removes the kill-switch route and the anchor address. Best-effort.
fn remove_ipv6_blackhole() {
    crate::log_msg("Removing IPv6 kill-switch route".into());
    let _ = netsh(&[
        "interface",
        "ipv6",
        "delete",
        "route",
        "::/0",
        ADAPTER_NAME,
        V6_GW,
    ]);
    let _ = netsh(&[
        "interface",
        "ipv6",
        "delete",
        "address",
        &format!("interface={ADAPTER_NAME}"),
        V6_ADDR,
    ]);
}

// ――― Adapter DNS pinning ―――――――――――――――――――――――――――――――――――――――――――――
// Pinning the adapter to known resolvers makes every query take exactly one
// deterministic path and closes the gap where interface-bound resolver
// traffic could escape through the physical NIC's own DNS config.

/// Sets the pinned resolvers on the tunnel adapter, then validates by reading
/// them back. Only called when the UDP relay is up — in TCP-only fallback the
/// adapter DNS is left untouched so resolution keeps working via physical.
fn apply_adapter_dns() -> anyhow::Result<()> {
    crate::log_msg(format!(
        "Pinning adapter DNS: {TUNNEL_DNS_PRIMARY}, {TUNNEL_DNS_SECONDARY}"
    ));
    netsh(&[
        "interface",
        "ip",
        "set",
        "dnsservers",
        &format!("name={ADAPTER_NAME}"),
        "source=static",
        &format!("address={TUNNEL_DNS_PRIMARY}"),
        "register=primary",
        "validate=no",
    ])?;
    netsh(&[
        "interface",
        "ip",
        "add",
        "dnsservers",
        &format!("name={ADAPTER_NAME}"),
        &format!("address={TUNNEL_DNS_SECONDARY}"),
        "index=2",
        "validate=no",
    ])?;
    // Validate: read back what the stack actually accepted.
    if let Ok(o) = Command::new("netsh")
        .args(["interface", "ip", "show", "dnsservers", &format!("name={ADAPTER_NAME}")])
        .output()
    {
        let text = String::from_utf8_lossy(&o.stdout);
        let ok = text.contains(TUNNEL_DNS_PRIMARY);
        crate::log_msg(format!("Adapter DNS validate: primary present = {ok}"));
    }
    Ok(())
}

/// Clears the static entries so no stale resolver survives disconnect.
fn clear_adapter_dns() {
    let _ = netsh(&[
        "interface",
        "ip",
        "delete",
        "dnsservers",
        &format!("name={ADAPTER_NAME}"),
        "all",
    ]);
}

/// IPv4 protocol numbers the tunnel routes specially.
const IPPROTO_TCP: u8 = 6;
const IPPROTO_UDP: u8 = 17;

/// Returns the IPv4 protocol byte (6 = TCP, 17 = UDP) for a packet that looks
/// like IPv4 with a full header, else `None`.
///
/// This is the classifier that decides what the tunnel captures. It is
/// unit-tested because it underpins the leak guarantee: UDP traffic — DNS,
/// STUN/TURN (WebRTC), mDNS — must be recognised so it is tunneled instead of
/// escaping onto the physical link.
pub(crate) fn ipv4_proto(bytes: &[u8]) -> Option<u8> {
    if bytes.len() > 9 && (bytes[0] >> 4) == 4 {
        Some(bytes[9])
    } else {
        None
    }
}

/// True if `bytes` is an IPv4/TCP segment carrying a SYN without ACK.
pub(crate) fn is_tcp_syn(bytes: &[u8]) -> bool {
    if ipv4_proto(bytes) != Some(IPPROTO_TCP) || bytes.len() < 20 {
        return false;
    }
    let ihl = (bytes[0] & 0x0f) as usize * 4;
    if ihl < 20 || bytes.len() < ihl + 14 {
        return false;
    }
    let flags = bytes[ihl + 13];
    (flags & 0x02) != 0 && (flags & 0x10) == 0
}

/// Extracts the IPv4 address from an adb device string like
/// `10.116.113.11:5555` or `10.116.113.11`.
fn device_ipv4(device: &str) -> Option<std::net::Ipv4Addr> {
    let host = device.split(':').next()?;
    host.parse::<std::net::Ipv4Addr>().ok()
}

/// Adds /32 host routes for every entry in the comma-separated filter list so
/// those destinations bypass the tunnel and keep using the physical network.
/// Each entry may be an IPv4 address or a hostname (resolved here).
pub fn add_host_exclusions(filter_hosts: &str) -> anyhow::Result<()> {
    for host in filter_hosts.split(',').map(str::trim).filter(|h| !h.is_empty()) {
        let ip: std::net::Ipv4Addr = if let Ok(ip) = host.parse() {
            ip
        } else {
            match format!("{}:0", host).to_socket_addrs() {
                Ok(mut addrs) => match addrs.next() {
                    Some(addr) => match addr.ip() {
                        IpAddr::V4(v4) => v4,
                        _ => continue,
                    },
                    None => continue,
                },
                Err(_) => continue,
            }
        };
        let physical = default_physical_gateway(&ip);
        if let Some((gw, idx)) = physical {
            crate::log_msg(format!("Filter route: {ip}/32 via {gw} if {idx}"));
            let status = Command::new("route")
                .args([
                    "add",
                    &ip.to_string(),
                    "mask",
                    "255.255.255.255",
                    &gw,
                    "if",
                    &idx,
                    "metric",
                    "1",
                ])
                .status()
                .map_err(|e| anyhow::anyhow!("route add {ip}/32 failed: {e}"))?;
            if !status.success() {
                crate::log_msg(format!("Filter route add {ip}/32 failed (exit {status})"));
            }
        }
    }
    Ok(())
}

/// Removes the /32 host routes that were added for the filter list.
pub fn remove_host_exclusions(filter_hosts: &str) {
    for host in filter_hosts.split(',').map(str::trim).filter(|h| !h.is_empty()) {
        let ip: std::net::Ipv4Addr = if let Ok(ip) = host.parse() {
            ip
        } else if let Ok(mut addrs) = format!("{}:0", host).to_socket_addrs() {
            match addrs.next() {
                Some(addr) => match addr.ip() {
                    IpAddr::V4(v4) => v4,
                    _ => continue,
                },
                None => continue,
            }
        } else {
            continue;
        };
        crate::log_msg(format!("Removing filter route: {ip}/32"));
        let _ = Command::new("route")
            .args(["delete", &ip.to_string(), "mask", "255.255.255.255"])
            .status();
    }
}

// ――― DNS resolution mode (DNS_MODE) ――――――――――――――――――――――――――――――――――
// mode 0 ("server"): the adapter's pinned resolvers are reached through the
//   tunnel, so queries exit at the phone.
// mode 1 ("client"): those same resolvers get /32 bypass routes via the
//   physical gateway, so Windows resolves through its own network while all
//   other traffic still tunnels.

/// Public resolvers pinned onto the tunnel adapter at `up`.
const TUNNEL_DNS_PRIMARY: &str = "1.1.1.1";
const TUNNEL_DNS_SECONDARY: &str = "8.8.8.8";

impl Tunnel {
/// Snapshots the tunnel resolvers + the physical path to each. Must run
/// before `add_default_route`, otherwise the tunnel's own metric=1
/// default would be picked as the "physical" gateway.
///
/// We deliberately pin the adapter to well-known public resolvers instead of
/// inheriting whatever the physical network handed out: every DNS query then
/// has exactly one deterministic path — through the tunnel (mode 0) or
/// directly via the bypass routes (mode 1).
pub fn capture_dns_servers(&mut self) {
    self.dns_routes.clear();
    for ip in [TUNNEL_DNS_PRIMARY, TUNNEL_DNS_SECONDARY] {
        let ip: std::net::Ipv4Addr = ip.parse().unwrap();
        if let Some((gw, idx)) = default_physical_gateway(&ip) {
            crate::log_msg(format!("Tunnel resolver {ip} via {gw} if {idx}"));
            self.dns_routes.push((ip, gw, idx));
        }
    }
    if self.dns_routes.is_empty() {
        crate::log_warn("no physical path found for tunnel resolvers".into());
    }
}

    /// Installs the ::/0 IPv6 blackhole route (kill-switch enabled).
    pub fn install_v6_blackhole(&mut self) {
        if self.v6_blackhole_active {
            return;
        }
        if let Err(e) = add_ipv6_blackhole() {
            crate::log_warn(format!("IPv6 kill-switch not applied: {e}"));
        }
        // Track attempted-state even on partial failure: `down` removal is
        // best-effort and a later enable must retry cleanly.
        self.v6_blackhole_active = true;
    }

    /// Removes the ::/0 IPv6 blackhole route (kill-switch disabled or down).
    pub fn remove_v6_blackhole(&mut self) {
        if !self.v6_blackhole_active {
            return;
        }
        remove_ipv6_blackhole();
        self.v6_blackhole_active = false;
    }

    /// Installs /32 bypass routes for the captured DNS servers (mode 1).
    pub fn install_dns_bypass(&mut self) {
        if self.dns_bypass_active || self.dns_routes.is_empty() {
            return;
        }
        crate::log_msg("DNS mode: client resolves (bypass routes on)".into());
        for (ip, gw, idx) in &self.dns_routes {
            let status = Command::new("route")
                .args([
                    "add",
                    &ip.to_string(),
                    "mask",
                    "255.255.255.255",
                    gw,
                    "if",
                    idx,
                    "metric",
                    "1",
                ])
                .status();
            if let Ok(s) = status {
                if !s.success() {
                    crate::log_msg(format!("DNS bypass route add {ip}/32 failed (exit {s})"));
                }
            }
        }
        self.dns_bypass_active = true;
    }

    /// Removes the DNS bypass routes (back to mode 0 behaviour).
    pub fn remove_dns_bypass(&mut self) {
        if !self.dns_bypass_active {
            return;
        }
        crate::log_msg("DNS mode: server resolves (bypass routes off)".into());
        for (ip, _, _) in &self.dns_routes {
            let _ = Command::new("route")
                .args(["delete", &ip.to_string(), "mask", "255.255.255.255"])
                .status();
        }
        self.dns_bypass_active = false;
    }
}

/// Finds the physical gateway + interface index currently used to reach `ip`.
fn default_physical_gateway(ip: &std::net::Ipv4Addr) -> Option<(String, String)> {
    let out = Command::new("route")
        .args(["print", &ip.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let subnet_prefix = format!("{}.", ip.octets()[0]);
    let mut gateway: Option<String> = None;
    let mut ifidx: Option<String> = None;
    for line in text.lines() {
        let t = line.trim();
        let mut cols = t.split_whitespace();
        let dest = cols.next()?;
        if dest.parse::<std::net::Ipv4Addr>().is_err() {
            continue;
        }
        let gw = cols.nth(1)?.to_string();
        let iface = cols.nth(0)?.to_string();
        if gw.is_empty() || gw.eq_ignore_ascii_case("On-link") {
            continue;
        }
        if dest.starts_with(&subnet_prefix) {
            gateway = Some(gw);
            ifidx = Some(iface);
            break;
        }
        if dest == "0.0.0.0" && gateway.is_none() {
            gateway = Some(gw);
            ifidx = Some(iface);
        }
    }
    Some((gateway?, ifidx?))
}

/// Adds a /32 host route for the adb device so the tunnel's default route does
/// not capture the adb connection itself. Without this, toggling the tunnel
/// kills the adb link (the adb traffic is routed back into the tunnel) and the
/// whole thing hangs.
///
/// Must run BEFORE the tunnel default route is installed, so the physical
/// gateway/interface for the device can still be discovered.
fn add_device_exclusion(device: &str) -> anyhow::Result<()> {
    let Some(ip) = device_ipv4(device) else {
        crate::log_msg(format!("Device exclusion skipped (no IPv4 in '{device}')"));
        return Ok(());
    };

    // Find the route currently used to reach the device, e.g.
    // `route print 10.116.113.11` → the best-match row. The "Interface"
    // column is the interface *IP*, not an index — so we let Windows pick the
    // interface automatically and only take the gateway.
    let out = Command::new("route")
        .args(["print", &ip.to_string()])
        .output()
        .map_err(|e| anyhow::anyhow!("route print failed: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);

    // Prefer the most specific route for the device subnet (e.g. 10.116.113.0),
    // fall back to the default route (0.0.0.0). If the best match is on-link
    // (device on the same subnet), use the device IP itself as the gateway —
    // route.exe treats that as an interface/on-link route.
    let subnet_prefix = format!("{}.", ip.octets()[0]);
    let mut gateway: Option<String> = None;
    let mut any_dest = false;
    for line in text.lines() {
        let t = line.trim();
        let mut cols = t.split_whitespace();
        let Some(dest) = cols.next() else { continue };
        if dest.parse::<std::net::Ipv4Addr>().is_err() {
            continue;
        }
        any_dest = true;
        let gw = cols.nth(1).unwrap_or("").to_string();
        if gw.is_empty() {
            continue;
        }
        if dest.starts_with(&subnet_prefix) {
            // Best match for the device: subnet route.
            if gw.eq_ignore_ascii_case("On-link") {
                gateway = Some(ip.to_string());
            } else {
                gateway = Some(gw);
            }
            break;
        }
        if dest == "0.0.0.0" && gateway.is_none() {
            if !gw.eq_ignore_ascii_case("On-link") {
                gateway = Some(gw);
            }
        }
    }
    if !any_dest || gateway.is_none() {
        crate::log_msg(format!("Device exclusion skipped (no route to {ip})"));
        return Ok(());
    }

    let gw = gateway.unwrap();
    crate::log_msg(format!("Adding device exclusion: {ip}/32 via {gw}"));
    let add = |gw: &str| {
        Command::new("route")
            .args([
                "add",
                &ip.to_string(),
                "mask",
                "255.255.255.255",
                gw,
                "metric",
                "1",
            ])
            .status()
    };
    if let Ok(st) = add(&gw) {
        if st.success() {
            crate::log_msg(format!("Device exclusion route added: {ip}/32"));
            return Ok(());
        }
    }
    // Route may already exist from a previous run; delete then retry.
    let _ = Command::new("route")
        .args(["delete", &ip.to_string(), "mask", "255.255.255.255"])
        .status();
    let st = add(&gw).map_err(|e| anyhow::anyhow!("route add failed: {e}"))?;
    if !st.success() {
        anyhow::bail!("route add {ip}/32 failed (exit {st})");
    }
    crate::log_msg(format!("Device exclusion route added: {ip}/32"));
    Ok(())
}

fn remove_device_exclusion(device: &str) {
    let Some(ip) = device_ipv4(device) else {
        return;
    };
    crate::log_msg(format!("Removing device exclusion route: {ip}/32"));
    let _ = Command::new("route")
        .args(["delete", &ip.to_string(), "mask", "255.255.255.255"])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal IPv4 packet with the given protocol, `ihl_words` words long
    /// (5 = 20-byte header, 6 = 24-byte header with options).
    fn ipv4(proto: u8, ihl_words: u8) -> Vec<u8> {
        let hdr = ihl_words as usize * 4;
        let mut b = vec![0u8; hdr + 20];
        b[0] = 0x40 | ihl_words;
        b[9] = proto;
        b
    }

    /// IPv4/TCP packet with `flags` in the TCP header.
    fn tcp(ihl_words: u8, flags: u8) -> Vec<u8> {
        let hdr = ihl_words as usize * 4;
        let mut b = vec![0u8; hdr + 20];
        b[0] = 0x40 | ihl_words;
        b[9] = IPPROTO_TCP;
        b[hdr + 12] = 0x50; // data offset 5
        b[hdr + 13] = flags;
        b
    }

    // ── capture classification: what the tunnel must never let escape ──────

    #[test]
    fn udp_is_classified_as_tunnel_captured() {
        // Plain UDP, DNS (53), STUN/WebRTC (19302), mDNS (5353) — all UDP.
        for _port in [53u16, 19302, 5353, 443] {
            assert_eq!(ipv4_proto(&ipv4(IPPROTO_UDP, 5)), Some(IPPROTO_UDP));
        }
    }

    #[test]
    fn tcp_is_classified_as_tcp() {
        assert_eq!(ipv4_proto(&ipv4(IPPROTO_TCP, 5)), Some(IPPROTO_TCP));
    }

    #[test]
    fn ipv6_and_short_packets_are_not_ipv4() {
        // IPv6 header (version nibble 6) must not be treated as IPv4.
        let mut v6 = vec![0u8; 40];
        v6[0] = 0x60;
        assert_eq!(ipv4_proto(&v6), None);
        // Too short to contain the protocol byte.
        assert_eq!(ipv4_proto(&[0x45u8; 9]), None);
        assert_eq!(ipv4_proto(&[]), None);
        // ARP / other EtherType payloads are not IP either.
        let mut arp = vec![0u8; 28];
        arp[0] = 0x00;
        assert_eq!(ipv4_proto(&arp), None);
    }

    #[test]
    fn tcp_syn_detection() {
        const SYN: u8 = 0x02;
        const ACK: u8 = 0x10;
        assert!(is_tcp_syn(&tcp(5, SYN)), "bare SYN");
        assert!(is_tcp_syn(&tcp(6, SYN)), "SYN with IP options");
        assert!(!is_tcp_syn(&tcp(5, SYN | ACK)), "SYN+ACK is not a new connection");
        assert!(!is_tcp_syn(&tcp(5, ACK)), "ACK");
        assert!(!is_tcp_syn(&tcp(5, 0)), "no flags");
        assert!(!is_tcp_syn(&ipv4(IPPROTO_UDP, 5)), "UDP is never a TCP SYN");
        // Header claims more than the packet holds.
        let mut short = tcp(5, SYN);
        short.truncate(25);
        assert!(!is_tcp_syn(&short));
    }

    // ── adb device string parsing (used for the device /32 exclusion) ──────

    #[test]
    fn device_ipv4_parses_ip_and_host_port() {
        assert_eq!(device_ipv4("10.116.113.11:5555"), Some("10.116.113.11".parse().unwrap()));
        assert_eq!(device_ipv4("10.116.113.11"), Some("10.116.113.11".parse().unwrap()));
    }

    #[test]
    fn device_ipv4_rejects_serials_and_garbage() {
        assert_eq!(device_ipv4("emulator-5554"), None);
        assert_eq!(device_ipv4(""), None);
        assert_eq!(device_ipv4("localhost:5555"), None);
    }

    // ── downlink drain: a partly-full tx ring must never drop bytes ────────

    #[test]
    fn drain_pending_keeps_bytes_the_sink_refused() {
        // Two 64 KiB chunks, as the bridge task delivers them.
        let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
        pending.push_back(vec![0xAA; 65536]);
        pending.push_back(vec![0xBB; 65536]);
        let mut off = 0usize;

        // A sink that takes at most 1500 bytes per call — a nearly full ring.
        let mut sink: Vec<u8> = Vec::new();
        let mut ticks = 0;
        while !pending.is_empty() {
            ticks += 1;
            assert!(ticks < 1000, "drain must make progress");
            drain_pending(&mut pending, &mut off, |b| {
                let take = b.len().min(1500);
                sink.extend_from_slice(&b[..take]);
                take
            });
        }

        // Every byte arrived, exactly once, in order.
        assert_eq!(sink.len(), 131_072, "every queued byte must reach the sink");
        assert!(sink[..65536].iter().all(|&b| b == 0xAA), "first chunk, in order");
        assert!(sink[65536..].iter().all(|&b| b == 0xBB), "second chunk, in order");
    }

    #[test]
    fn drain_pending_retains_everything_when_sink_is_full() {
        let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
        pending.push_back(vec![1, 2, 3, 4]);
        let mut off = 0usize;

        drain_pending(&mut pending, &mut off, |_| 0);

        assert_eq!(off, 0);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], vec![1, 2, 3, 4]);
    }

    #[test]
    fn drain_pending_pops_fully_written_chunks() {
        let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
        pending.push_back(vec![1, 2]);
        pending.push_back(vec![3]);
        let mut off = 0usize;
        let mut out: Vec<u8> = Vec::new();

        drain_pending(&mut pending, &mut off, |b| {
            out.extend_from_slice(b);
            b.len()
        });

        assert!(pending.is_empty());
        assert_eq!(off, 0, "offset resets when the queue empties");
        assert_eq!(out, vec![1, 2, 3]);
    }
}


