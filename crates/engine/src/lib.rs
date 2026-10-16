mod config;
mod proxy;
mod tunnel;

use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::Mutex;
use std::time::Duration;

static LOG: once_cell::sync::Lazy<Mutex<Vec<String>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(Vec::new()));
static TUNNEL: once_cell::sync::Lazy<Mutex<Option<tunnel::Tunnel>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));
static STATUS: once_cell::sync::Lazy<Mutex<String>> =
    once_cell::sync::Lazy::new(|| Mutex::new("Ready".into()));
static DEVICE: once_cell::sync::Lazy<Mutex<String>> =
    once_cell::sync::Lazy::new(|| Mutex::new(String::new()));
static CONNECTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static MONITOR_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// 0 = DNS queries exit via the phone (default), 1 = client resolves through
/// its own network (the adapter's pinned resolvers get physical bypass
/// routes).
static DNS_MODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// 1 = apply the ::/0 IPv6 kill-switch while the tunnel runs (default),
/// 0 = leave IPv6 on the physical network (leaks AAAA/DNS-v6/WebRTC!).
static IPV6_BLOCK: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);
/// Adb forward + proxy port. Default 1092.
static PROXY_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(1092);
/// Total bytes pushed from the app into the tunnel (uplink).
static UPLINK_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Total bytes pulled from the tunnel back to the app (downlink).
static DOWNLINK_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Hosts that bypass the tunnel (comma-separated domains/IPs from settings).
static FILTER_HOSTS: once_cell::sync::Lazy<Mutex<String>> =
    once_cell::sync::Lazy::new(|| Mutex::new(String::new()));

pub fn add_uplink(n: u64) {
    UPLINK_BYTES.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

pub fn add_downlink(n: u64) {
    DOWNLINK_BYTES.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: once_cell::sync::OnceCell<tokio::runtime::Runtime> = once_cell::sync::OnceCell::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
}

fn set_status(s: String) { if let Ok(mut st) = STATUS.lock() { *st = s; } }
fn get_status() -> String { STATUS.lock().unwrap().clone() }

fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs() % 86400;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    format!("[{:02}:{:02}:{:02}]", h, m, s)
}

/// INFO level log (default).
fn log_msg(s: String) {
    log_line("INFO", &s);
}

/// ERROR level log.
fn log_err(s: String) {
    log_line("ERROR", &s);
}

/// WARN level log.
fn log_warn(s: String) {
    log_line("WARN", &s);
}

fn log_line(level: &str, s: &str) {
    let line = format!("{} {} {}", timestamp(), level, s);
    println!("{}", line);
    if let Ok(mut b) = LOG.lock() {
        b.push(line);
        let c = b.len().saturating_sub(5000);
        if c > 0 { b.drain(0..c); }
    }
}

/// Starts the background liveness monitor. Probes the WebSocket proxy every
/// ~2s while the tunnel is up; the moment the probe fails it flips the status
/// to "Offline: connection lost". The tunnel is deliberately NOT torn down:
/// routes, adapter and exclusions stay installed so Windows remains on the
/// tunnel (fail-closed, no connectivity) until the user presses Disconnect.
/// Started on every path that brings the tunnel up (start and resume).
fn start_monitor() {
    if MONITOR_RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        log_warn("liveness monitor already running, skipping".to_string());
        return;
    }
    log_msg("liveness monitor started (2s interval)".to_string());
    let device = DEVICE.lock().unwrap().clone();
    let cfg = config::Config { device, port: PROXY_PORT.load(std::sync::atomic::Ordering::SeqCst), ..Default::default() };
    runtime().spawn(async move {
        while MONITOR_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if !MONITOR_RUNNING.load(std::sync::atomic::Ordering::SeqCst) { break; }
            let r = tokio::time::timeout(Duration::from_secs(3), proxy::proxy_alive(&cfg.bind, cfg.port)).await;
            match r {
                Ok(Ok(())) => {}
                _ => {
                    log_err("liveness: connection lost, keeping tunnel (fail-closed)".to_string());
                    set_status("Offline: connection lost".into());
                    CONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
                    MONITOR_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
                    // Do NOT tear the tunnel down. A dead proxy must not make
                    // traffic silently fall back out the physical network, so
                    // the default route, DNS pinning and adapter stay in place:
                    // Windows remains on the tunnel with no connectivity until
                    // the user presses Disconnect (which calls `down`).
                    break;
                }
            }
        }
        log_msg("liveness monitor stopped".to_string());
    });
}

fn stop_monitor() {
    if MONITOR_RUNNING.swap(false, std::sync::atomic::Ordering::SeqCst) {
        log_msg("stopping liveness monitor".to_string());
    }
    CONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Checks that the WebSocket proxy is reachable on the forwarded port.
/// The caller (C# AdbService) is responsible for adb forward + proxy startup.
async fn ensure_proxy_up(cfg: &config::Config) -> anyhow::Result<()> {
    log_msg(format!("ensure_proxy_up: probing ws://{}:{}", cfg.bind, cfg.port));
    let r = tokio::time::timeout(Duration::from_secs(3), proxy::proxy_alive(&cfg.bind, cfg.port)).await;
    match &r {
        Ok(Ok(())) => log_msg(format!("proxy alive: ws://{}:{}", cfg.bind, cfg.port)),
        Ok(Err(e)) => log_warn(format!("proxy probe failed: {e}")),
        Err(_) => log_warn(format!("proxy probe timed out after 3s")),
    }
    if r.ok().and_then(|r| r.ok()).is_none() {
        let msg = format!("Proxy not reachable on {}:{}", cfg.bind, cfg.port);
        log_err(msg.clone());
        anyhow::bail!(msg);
    }
    Ok(())
}

fn write_to_buf(buf: *mut c_char, buf_len: usize, s: &str) -> usize {
    if buf.is_null() || buf_len == 0 { return s.len(); }
    let bytes = s.as_bytes();
    let copy = bytes.len().min(buf_len - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, copy);
        *buf.add(copy) = 0;
    }
    copy
}

// ── C FFI exports ────────────────────────────────────────────────────

/// Initializes the runtime. Must be called once before any other function.
/// Returns 0 on success.
#[no_mangle]
pub extern "C" fn top_init() -> i32 {
    let _ = runtime();
    set_status("Ready".into());
    log_msg("AdbTether engine initialized".to_string());
    log_msg(format!("Runtime ready, status buffer live"));
    0
}

/// Sets the active device. `device` must be a null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn top_set_device(device: *const c_char) {
    if let Ok(s) = unsafe { CStr::from_ptr(device) }.to_str() {
        if let Ok(mut d) = DEVICE.lock() { *d = s.to_string(); }
    }
}

/// Sets DNS resolution mode: 0 = server (device) resolves, 1 = client (Windows).
/// Applied immediately: on a live tunnel the system-DNS bypass routes are
/// installed/removed on the fly.
#[no_mangle]
pub extern "C" fn top_set_dns_mode(mode: i32) {
    DNS_MODE.store(mode, std::sync::atomic::Ordering::SeqCst);
    let enable = mode == 1;
    if let Ok(mut guard) = TUNNEL.lock() {
        if let Some(t) = guard.as_mut() {
            if !t.suspended {
                if enable { t.install_dns_bypass(); } else { t.remove_dns_bypass(); }
            }
        }
    }
}

/// Returns the current DNS resolution mode: 0 = server, 1 = client.
pub fn dns_mode() -> i32 {
    DNS_MODE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Enables/disables the IPv6 kill-switch. Applied immediately on a live,
/// non-suspended tunnel; otherwise takes effect at the next `up`.
#[no_mangle]
pub extern "C" fn top_set_ipv6_block(v: i32) {
    IPV6_BLOCK.store(v, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut guard) = TUNNEL.lock() {
        if let Some(t) = guard.as_mut() {
            if !t.suspended {
                if v == 1 { t.install_v6_blackhole(); } else { t.remove_v6_blackhole(); }
            }
        }
    }
    log_msg(format!("ipv6 block set to {}", v == 1));
}

/// Returns 1 when the IPv6 kill-switch is enabled (default).
#[no_mangle]
pub extern "C" fn top_get_ipv6_block() -> i32 {
    IPV6_BLOCK.load(std::sync::atomic::Ordering::SeqCst)
}

/// Tunnel-internal accessor for the kill-switch setting.
pub fn ipv6_block_enabled() -> bool {
    IPV6_BLOCK.load(std::sync::atomic::Ordering::SeqCst) == 1
}

/// Returns the current DNS resolution mode: 0 = server, 1 = client.
#[no_mangle]
pub extern "C" fn top_get_dns_mode() -> i32 {
    DNS_MODE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Sets the adb forward + proxy port.
#[no_mangle]
pub extern "C" fn top_set_port(port: u16) {
    PROXY_PORT.store(port, std::sync::atomic::Ordering::SeqCst);
}

/// Returns the current adb forward + proxy port.
#[no_mangle]
pub extern "C" fn top_get_port() -> u16 {
    PROXY_PORT.load(std::sync::atomic::Ordering::SeqCst)
}

/// Sets the comma-separated list of hosts (domains/IPs) that bypass the tunnel.
/// Applied immediately: on a live tunnel the /32 bypass routes are swapped, so
/// removed hosts stop bypassing and added hosts start bypassing right away.
#[no_mangle]
pub unsafe extern "C" fn top_set_filter_hosts(hosts: *const c_char) {
    if hosts.is_null() {
        if let Ok(mut h) = FILTER_HOSTS.lock() { h.clear(); }
        apply_filter_hosts_live("");
        return;
    }
    if let Ok(s) = unsafe { CStr::from_ptr(hosts) }.to_str() {
        if let Ok(mut h) = FILTER_HOSTS.lock() { *h = s.to_string(); }
        apply_filter_hosts_live(s);
    }
}

/// Swaps the host-exclusion routes on a live (non-suspended) tunnel so filter
/// list edits take effect without a reconnect. Diff-based: routes of hosts
/// present in both lists are left untouched (no connection blips), removed
/// hosts stop bypassing, added hosts start bypassing. Keeps
/// `Tunnel.filter_hosts` in sync with the routes actually installed, which
/// also fixes the leak where `down()` could not remove routes of hosts that
/// had already been dropped from the list.
fn apply_filter_hosts_live(new_hosts: &str) {
    let mut guard = match TUNNEL.lock() { Ok(g) => g, Err(_) => return };
    let t = match guard.as_mut() {
        Some(t) if !t.suspended => t,
        _ => return, // no active tunnel: routes are already absent
    };
    let parse = |s: &str| -> Vec<String> {
        s.split(',').map(str::trim).filter(|h| !h.is_empty())
            .map(String::from).collect()
    };
    let old_list = parse(&std::mem::take(&mut t.filter_hosts));
    let new_list = parse(new_hosts);
    let to_remove: Vec<String> = old_list.iter()
        .filter(|h| !new_list.contains(h)).cloned().collect();
    let to_add: Vec<String> = new_list.iter()
        .filter(|h| !old_list.contains(h)).cloned().collect();

    t.filter_hosts = new_hosts.to_string();
    if !to_remove.is_empty() {
        tunnel::remove_host_exclusions(&to_remove.join(","));
    }
    if !to_add.is_empty() {
        if let Err(e) = tunnel::add_host_exclusions(&to_add.join(",")) {
            log_err(format!("filter swap: adding exclusions failed: {e}"));
        }
    }
    log_msg(format!(
        "filter hosts applied live: +{} -{} (total {})",
        to_add.len(), to_remove.len(), new_list.len()
    ));
}

/// Writes the current filter host list (comma-separated) to `buf`.
#[no_mangle]
pub extern "C" fn top_get_filter_hosts(buf: *mut c_char, buf_len: usize) -> usize {
    let s = FILTER_HOSTS.lock().unwrap().clone();
    write_to_buf(buf, buf_len, &s)
}

/// Starts a connection to the proxy. Runs asynchronously.
/// Returns 0 immediately. Poll top_get_status() for result.
#[no_mangle]
pub extern "C" fn top_connect() -> i32 {
    let device = DEVICE.lock().unwrap().clone();
    let cfg = config::Config { device, port: PROXY_PORT.load(std::sync::atomic::Ordering::SeqCst), ..Default::default() };
    log_msg(format!("top_connect: device='{}' port={}", cfg.device, cfg.port));
    set_status("Connecting...".into());
    runtime().spawn(async move {
        let result = ensure_proxy_up(&cfg).await;
        match result {
            Ok(()) => {
                CONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                set_status("Connected".into());
                log_msg("connected, starting liveness monitor".to_string());
                start_monitor();
            }
            Err(e) => {
                CONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
                set_status(format!("Error: {e}"));
                log_err(format!("connect failed: {e}"));
            }
        }
    });
    0
}

/// Disconnects from the proxy.
#[no_mangle]
pub extern "C" fn top_disconnect() {
    log_msg("disconnect requested".to_string());
    stop_monitor();
    set_status("Disconnected".into());
    log_msg("disconnected".to_string());
}

/// Returns 1 while the proxy is connected and alive, 0 otherwise.
#[no_mangle]
pub extern "C" fn top_is_connected() -> i32 {
    if CONNECTED.load(std::sync::atomic::Ordering::SeqCst) { 1 } else { 0 }
}

/// Toggles the tunnel. Returns 0 immediately. Poll top_get_status().
/// Disconnect (tunnel up -> down) only suspends: the wintun adapter stays
/// open so a subsequent connect resumes instantly. The adapter is fully
/// removed only on `top_cleanup` (app shutdown).
#[no_mangle]
pub extern "C" fn top_toggle_tunnel() -> i32 {
    let device = DEVICE.lock().unwrap().clone();
    let cfg = config::Config { device, port: PROXY_PORT.load(std::sync::atomic::Ordering::SeqCst), ..Default::default() };

    let mut guard = TUNNEL.lock().unwrap();
    match &*guard {
        Some(t) if t.suspended => {
            // Resume: reuse the existing adapter for an instant reconnect.
            log_msg("tunnel toggle: resuming".to_string());
            set_status("Tunnel resuming...".into());
            let mut tunnel = guard.take().unwrap();
            let rt = runtime().handle().clone();
            drop(guard);
            runtime().spawn(async move {
                let port = PROXY_PORT.load(std::sync::atomic::Ordering::SeqCst);
                let addr = format!("127.0.0.1:{port}").parse().unwrap();
                let filter_hosts = FILTER_HOSTS.lock().unwrap().clone();
                match ensure_proxy_up(&cfg).await {
                    Ok(()) => match tunnel.up(addr, &filter_hosts, rt).await {
                        Ok(()) => {
                            let udp = tunnel.udp_supported;
                            *TUNNEL.lock().unwrap() = Some(tunnel);
                            CONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                            let st = if udp { "Tunnel active - all traffic via VPN".to_string() }
                                     else { "Tunnel active (TCP only)".to_string() };
                            log_msg(format!("tunnel resumed: udp={udp}"));
                            set_status(st);
                            start_monitor();
                        }
                        Err(e) => {
                            log_err(format!("tunnel resume failed: {e}"));
                            *TUNNEL.lock().unwrap() = Some(tunnel);
                            CONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
                            set_status(format!("Tunnel error: {e}"));
                        }
                    },
                    Err(e) => {
                        log_err(format!("tunnel resume aborted: {e}"));
                        *TUNNEL.lock().unwrap() = Some(tunnel);
                        set_status(format!("No proxy: {e}"));
                    }
                }
            });
            0
        }
        Some(_) => {
            // Suspend: stop session + routes, keep the adapter.
            log_msg("tunnel toggle: suspending".to_string());
            stop_monitor();
            let mut t = guard.take().unwrap();
            drop(guard); // release the TUNNEL lock before down() / relock
            t.down();
            *TUNNEL.lock().unwrap() = Some(t);
            CONNECTED.store(false, std::sync::atomic::Ordering::SeqCst);
            set_status("Tunnel suspended".into());
            log_msg("tunnel suspended".to_string());
            0
        }
        None => {
            // First connect: open the adapter, then bring the tunnel up.
            log_msg(format!("tunnel toggle: starting (device='{}' port={})", cfg.device, cfg.port));
            set_status("Tunnel starting...".into());
            let rt = runtime().handle().clone();
            let device = DEVICE.lock().unwrap().clone();
            match tunnel::Tunnel::open(&device) {
                Ok(mut tunnel) => {
                    runtime().spawn(async move {
                        let port = PROXY_PORT.load(std::sync::atomic::Ordering::SeqCst);
                        let addr = format!("127.0.0.1:{port}").parse().unwrap();
                        let filter_hosts = FILTER_HOSTS.lock().unwrap().clone();
                        match ensure_proxy_up(&cfg).await {
                            Ok(()) => {
                                log_msg(format!("tunnel start: proxy={addr} filter='{filter_hosts}'"));
                                match tunnel.up(addr, &filter_hosts, rt).await {
                                    Ok(()) => {
                                        let udp = tunnel.udp_supported;
                                        *TUNNEL.lock().unwrap() = Some(tunnel);
                                        CONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                                        let st = if udp { "Tunnel active - all traffic via VPN".to_string() }
                                                 else { "Tunnel active (TCP only)".to_string() };
                                        log_msg(format!("tunnel active: udp={udp}"));
                                        set_status(st);
                                        start_monitor();
                                    }
                                    Err(e) => {
                                        log_err(format!("tunnel start failed: {e}"));
                                        set_status(format!("Tunnel error: {e}"));
                                    }
                                }
                            }
                            Err(e) => {
                                log_err(format!("tunnel start aborted: {e}"));
                                set_status(format!("No proxy: {e}"));
                            }
                        }
                    });
                }
                Err(e) => {
                    log_err(format!("tunnel open failed: {e}"));
                    set_status(format!("Tunnel error: {e}"));
                }
            }
            0
        }
    }
}

/// Writes the current status string to `buf`. Returns bytes written.
#[no_mangle]
pub extern "C" fn top_get_status(buf: *mut c_char, buf_len: usize) -> usize {
    let s = get_status();
    write_to_buf(buf, buf_len, &s)
}

/// Writes cumulative traffic counters as JSON to `buf`:
/// `{"uplink_bytes":N,"downlink_bytes":N}`. Returns bytes written.
#[no_mangle]
pub extern "C" fn top_get_stats(buf: *mut c_char, buf_len: usize) -> usize {
    let json = serde_json::json!({
        "uplink_bytes": UPLINK_BYTES.load(std::sync::atomic::Ordering::Relaxed),
        "downlink_bytes": DOWNLINK_BYTES.load(std::sync::atomic::Ordering::Relaxed),
    })
    .to_string();
    write_to_buf(buf, buf_len, &json)
}

/// Resets the traffic counters to zero.
#[no_mangle]
pub extern "C" fn top_reset_stats() {
    UPLINK_BYTES.store(0, std::sync::atomic::Ordering::Relaxed);
    DOWNLINK_BYTES.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Writes new log lines (JSON array) to `buf`. Returns bytes written.
#[no_mangle]
pub extern "C" fn top_get_log(buf: *mut c_char, buf_len: usize) -> usize {
    let lines: Vec<String> = LOG.lock().map(|mut b| b.drain(..).collect()).unwrap_or_default();
    let json = serde_json::to_string(&lines).unwrap_or_default();
    write_to_buf(buf, buf_len, &json)
}

/// Clears the log buffer.
#[no_mangle]
pub extern "C" fn top_clear_log() {
    if let Ok(mut b) = LOG.lock() { b.clear(); }
}

/// Returns 1 if the tunnel is active (up and not suspended), 0 otherwise.
#[no_mangle]
pub extern "C" fn top_tunnel_active() -> i32 {
    let g = TUNNEL.lock().unwrap();
    match &*g {
        Some(t) if !t.suspended => 1,
        _ => 0,
    }
}

/// Full teardown that keeps the adapter: removes routes, DNS pinning and the
/// static adapter IP so Windows leaves the tunnel completely. Called when the
/// UI app exits (the service detects its last RPC client disconnecting), so
/// closing the app never leaves the PC routed into a dead tunnel. The adapter
/// is kept, so the next connect resumes instantly. No-op without a tunnel.
#[no_mangle]
pub extern "C" fn top_teardown() {
    log_msg("teardown requested".to_string());
    stop_monitor();
    let taken = TUNNEL.lock().ok().and_then(|mut g| g.take());
    if let Some(mut t) = taken {
        t.teardown();
        if let Ok(mut g) = TUNNEL.lock() {
            *g = Some(t);
        }
    }
    log_msg("teardown complete".to_string());
}

/// Shuts down the tunnel and deletes the adapter. Called when the Windows
/// service stops (MSI uninstall, manual stop, machine shutdown). The UI app
/// exiting reaches `top_teardown` instead (routes/DNS/IP cleared, adapter
/// kept), so a normal app close still leaves the network out of the tunnel
/// while the auto-start service keeps the adapter for a fast reconnect.
/// Recreated instantly on the next connect via the Adapter::open fast path.
#[no_mangle]
pub extern "C" fn top_cleanup() {
    log_msg("cleanup requested".to_string());
    stop_monitor();
    if let Some(t) = TUNNEL.lock().unwrap().take() { t.destroy(); }
    log_msg("cleanup complete".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn proxy_port_defaults_to_1092() {
        assert_eq!(top_get_port(), 1092);
    }

    #[test]
    fn ipv6_killswitch_defaults_on() {
        // Fail-closed by default: the ::/0 blackhole is ON so AAAA records,
        // DNS-over-IPv6 and WebRTC IPv6 candidates cannot leak to the physical
        // network while the tunnel is up.
        assert_eq!(top_get_ipv6_block(), 1);
        assert!(ipv6_block_enabled());
        top_set_ipv6_block(0);
        assert_eq!(top_get_ipv6_block(), 0);
        assert!(!ipv6_block_enabled());
        top_set_ipv6_block(1);
        assert!(ipv6_block_enabled());
    }

    #[test]
    fn dns_mode_defaults_to_server_resolution() {
        // 0 = the phone (server) resolves; the adapter is pinned to the tunnel
        // resolvers so lookups do not leak to the physical DNS.
        assert_eq!(top_get_dns_mode(), 0);
        assert_eq!(dns_mode(), 0);
        top_set_dns_mode(1); // no live tunnel => plain toggle, no netsh
        assert_eq!(top_get_dns_mode(), 1);
        top_set_dns_mode(0);
        assert_eq!(top_get_dns_mode(), 0);
    }

    #[test]
    fn write_to_buf_truncates_and_nul_terminates() {
        let mut buf = [0i8; 6];
        let n = write_to_buf(buf.as_mut_ptr(), buf.len(), "hello world");
        assert_eq!(n, 5);
        assert_eq!(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap(), "hello");
    }

    #[test]
    fn write_to_buf_null_or_zero_returns_len() {
        assert_eq!(write_to_buf(std::ptr::null_mut(), 0, "abc"), 3);
        let mut buf = [0i8; 4];
        assert_eq!(write_to_buf(buf.as_mut_ptr(), 0, "abc"), 3);
    }

    #[test]
    fn stats_json_roundtrip_and_reset() {
        top_reset_stats();
        add_uplink(10);
        add_downlink(20);
        let mut buf = vec![0i8; 128];
        let n = top_get_stats(buf.as_mut_ptr(), buf.len());
        assert!(n > 0);
        let json = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap().to_string();
        assert!(json.contains("\"uplink_bytes\":10"), "{json}");
        assert!(json.contains("\"downlink_bytes\":20"), "{json}");
        top_reset_stats();
        let mut buf2 = vec![0i8; 128];
        top_get_stats(buf2.as_mut_ptr(), buf2.len());
        let json2 = unsafe { CStr::from_ptr(buf2.as_ptr()) }.to_str().unwrap();
        assert!(json2.contains("\"uplink_bytes\":0"), "{json2}");
        assert!(json2.contains("\"downlink_bytes\":0"), "{json2}");
    }

    #[test]
    fn tunnel_inactive_without_tunnel() {
        assert_eq!(top_tunnel_active(), 0);
    }
}


