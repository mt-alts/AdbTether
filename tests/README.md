# AdbTether tests

Three layers, from fastest to most invasive.

| Layer | Where | Needs | Runs in CI |
|---|---|---|---|
| Rust unit + integration | `crates/**` (`#[cfg(test)]`, `crates/ws-proxy/tests`) | Rust only | yes |
| Runner | `test.ps1` | Rust (live part: admin + tunnel) | yes (Rust part) |
| Live leak checks | `tests/leak/Test-Leaks.ps1` | admin + a raised tunnel | no (skips) |

## Rust tests

```powershell
cargo test --workspace          # engine + ws-proxy tests
./test.ps1                      # same, plus a friendly summary
```

### What is covered

**Tunnel engine** (`crates/engine`, in-crate `#[cfg(test)]`)

* `tunnel::mod` - packet classification and the leak guarantee
  (`ipv4_proto`, `is_tcp_syn`) plus `device_ipv4` parsing. UDP (DNS, STUN,
  mDNS) **must** be classified as tunnel-captured; IPv6 is not IPv4 and is
  dropped by smoltcp.
* `tunnel::udp` - raw IPv4/UDP parsing (`parse_udp4`), reply rebuilding
  (`build_udp4` + checksum), and the flow-id demux table (`FlowTable`):
  unique/stable ids, zero-id skip on wraparound, oldest-entry eviction at the
  cap, shared-resolver no-crosstalk, legacy `flow_id=0` fallback. Includes
  direct DNS / STUN(WebRTC) / mDNS **capture** tests.
* `tunnel::ws` - the `connect_tcp` CONNECT request/byte-stream and the UDP
  datagram framing (`fid(2) FRAG(1) ATYP(1) addr port payload`), malformed
  frame skipping and buffer truncation, over a fake in-process server.
* `lib` - FFI/settings: default proxy port **1092**, IPv6 kill-switch default
  **on** + toggle, DNS-mode default **0** + toggle, `write_to_buf` truncation,
  stats JSON + reset.

**Phone-side proxy** (`crates/ws-proxy/tests/relay.rs`)

Spawns the real `ws-proxy` binary and drives it over a real loopback
WebSocket: TCP CONNECT round trip, CONNECT error for a dead target, and UDP
round trips for DNS (53), STUN/WebRTC, hostname (`ATYP 0x03`) targets and two
concurrent flows to the same resolver.

## Live leak checks

`tests/leak/Test-Leaks.ps1` inspects the running system. It **skips** (exit 0)
when it is not elevated or the `AdbTether` adapter is not up, so it is safe to
run anywhere.

```powershell
# elevated shell, tunnel up
./tests/leak/Test-Leaks.ps1
./tests/leak/Test-Leaks.ps1 -ExpectedPublicIp <phone's public IP>
```

It checks:

* **DNS leak** - the adapter is pinned to the tunnel resolvers
  (`1.1.1.1`, `8.8.8.8`) and resolution works through the tunnel, so lookups
  never fall back to the physical ISP resolver.
* **IPv6 leak** - the winning `::/0` route is the tunnel's fail-closed
  blackhole; no physical default route ties or beats it (otherwise AAAA,
  DNS-over-IPv6 and WebRTC IPv6 candidates could leak).
* **WebRTC leak** - a real STUN binding request is sent; the egress address
  must be public and, with `-ExpectedPublicIp`, must match the tunnel's egress.
