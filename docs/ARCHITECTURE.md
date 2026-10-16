# AdbTether — Project Documentation

## 1. Purpose

AdbTether **shares an Android phone's internet connection directly with a Windows PC**.

- **Scenario:** the PC has no internet or a slow one, while the phone has a good connection (cellular / Wi-Fi).
- **Solution:** an L3 tunnel is created, and all of the PC's traffic exits through the phone.
- **Constraints:** no root, no hotspot — a USB (ADB) or TCP connection is enough.
- **Reverse tethering:** phone → PC (the opposite of conventional tethering).

```
[PC app]  ──wintun/L3──►  [Windows service]  ──WebSocket──►  [Phone ws-proxy]  ──►  [Internet]
 WinUI 3 + Rust engine        C# SYSTEM                     Rust (Android)
```

---

## 2. Architecture

### Component diagram

```
Windows PC
├── AdbTether.exe — WinUI 3 desktop app (C#)
│     Settings page (port, filter list, IPv6 toggle) · device list (ADB discovery)
│     Connection flow (connect / suspend) · RPC client on 127.0.0.1:47012
│            ▲
│            │ RPC (TCP loopback)
│            ▼
├── AdbTetherService.exe — Windows service (C# SYSTEM)
│     EngineBridge FFI → AdbTether_core.dll · TcpServer RPC endpoint · lifecycle
│            ▲
│            │ FFI (P/Invoke)
│            ▼
└── AdbTether_core.dll — Rust engine
      wintun adapter "AdbTether" · smoltcp TCP/IP stack (user space) · tokio async runtime
      UDP relay (flow-id demultiplexing) · route management (netsh + route)
             │
             │ ADB forward / TCP
             ▼
Android phone
└── ws-proxy — Rust binary, pushed to the device
      WebSocket server (CONNECT + UDP relay); opens real connections from the phone's network
             │
             ▼
          Internet
```

### Data flow

```
PC app
    │  Start command (RPC)
    ▼
Service → Engine: wintun adapter "AdbTether" is created
    │
    ▼
Static IP (10.8.0.2/24) is assigned to the adapter
    │
    ▼
Default route added: 0.0.0.0/0 metric=1 → all traffic into the tunnel
    │
    ▼
smoltcp TCP stack + UDP relay are set up
    │
    ▼
Reader thread (wintun): TCP and UDP packets are split
    ├── TCP  → ws-proxy CONNECT pathway (real connection on the phone)
    └── UDP  → ws-proxy UDP-associate pathway (routed via the flow-id table)
```

### Shutdown flows (Disconnect / App exit / Service stop)

```
Disconnect (UI button) → top_toggle_tunnel() → Tunnel::down()
    ├─ session is closed
    ├─ worker threads are joined
    ├─ remove_default_route()
    ├─ remove_ipv6_blackhole()
    ├─ remove_dns_bypass()
    ├─ remove_host_exclusions()
    └─ remove_device_exclusion()
    ⇒ adapter + its IP are kept (fast resume); the on-device proxy stays alive

App exit → service sees its last RPC client disconnect → top_teardown()
    ├─ Tunnel::down()  (as above)
    └─ reset_ip()      (adapter address → DHCP)
    ⇒ the PC is fully off the tunnel; the adapter is kept

Service stops (MSI uninstall or manual stop) → top_cleanup() → Tunnel::destroy()
    ├─ down()          (as above)
    └─ the adapter is deleted
```

> **Connection lost is fail-closed.** When the liveness monitor detects a dead
> `ws-proxy` it only flips the status to `Offline: connection lost` and stops the
> monitor. Routes, DNS pinning and the adapter stay installed, so the PC remains
> on the tunnel with no connectivity until the user presses Disconnect — traffic
> never silently falls back to the physical network.

> **Note:** a plain Disconnect only suspends the tunnel — it removes the routes
> and closes the session but **keeps** the wintun adapter and the on-device proxy
> alive, so the next connect resumes instantly. Closing the app is stronger: the
> service notices the UI is gone and clears the adapter IP too (see above).

---

## 3. Features

### Core

| Feature | Description |
|---|---|
| **L3 tunnel** | All traffic (TCP + UDP) is routed out through the phone |
| **No hotspot needed** | Works over USB (ADB) or TCP; no physical cable required |
| **No root needed** | Android root is not required; ws-proxy is pushed to `/data/local/tmp` and started over `adb shell` |
| **Service architecture** | A SYSTEM service runs in the background; the tunnel survives closing the UI |
| **Adapter persistence** | The wintun adapter is kept even when the app is closed; it is removed on uninstall |
| **Fast reconnect** | The adapter stays ready (no cold start), so reconnecting the tunnel is naturally quick |

### Network

| Feature | Description |
|---|---|
| **Default route** | `0.0.0.0/0 metric=1` → all unicast traffic into the tunnel |
| **Host exclusions** | A domain/IP list is kept out of the tunnel (`/32` routes via the physical gateway) |
| **Live filter updates** | Hosts added/removed while the tunnel is active are reflected in routes immediately (diff-based swap) |
| **Device exclusion** | The ADB device IP is kept out of the tunnel (prevents the connection from dropping) |
| **DNS pinning** | The adapter is pinned to fixed resolvers (1.1.1.1, 8.8.8.8) via a physical-gateway bypass route |
| **DNS resolution mode** | Mode 0: resolve on the phone (default, exits through the tunnel); Mode 1: physical bypass (hidden option) |
| **IPv6 kill-switch** | All IPv6 traffic is dropped with a `::/0` blackhole route (default: on) |
| **IPv6 toggle** | Can be turned on/off from Settings; switches instantly on a live tunnel |

### UDP relay

| Feature | Description |
|---|---|
| **Flow-ID demux** | Each application socket gets a unique 16-bit ID (written into the SOCKS5 header's RSV field) |
| **Per-flow upstream socket** | The phone side creates a separate UDP socket per flow_id |
| **Reply routing** | Replies are routed by flow_id, so concurrent queries to the same target no longer collide |
| **8192-flow capacity** | Large-scale concurrency supported via ring eviction |
| **Legacy fallback** | `flow_id=0` (older servers) → the old target-keyed map behaviour |

### Application

| Feature | Description |
|---|---|
| **Settings page** | IPv6 toggle, proxy port, filter hosts list |
| **Filter list CRUD** | Add/remove, comma-separated list, live updates |
| **Theme support** | Light/Dark/System, persisted in settings (`AppearanceService`) |
| **ADB device management** | USB/TCP device discovery, forward, proxy start/stop |
| **Log viewer** | Real-time log stream, read from the buffer |
| **Update check** | Version check via GitHub Releases |

### Distribution

| Feature | Description |
|---|---|
| **MSI packaging** | Microsoft Installer format via WiX 5.0 |
| **Service install** | Declarative `ServiceInstall`/`ServiceControl` tables (SCM API) |
| **Silent install** | `msiexec /i AdbTether.msi /qn` |
| **Silent uninstall** | `msiexec /x AdbTether.msi /qn` |
| **Upgrade** | `MajorUpgrade` (removes the previous version automatically) |
| **AV cleanliness** | No PowerShell / sc.exe / external processes; minimal behaviour-based false positives |

---

## 4. Project structure

```
AdbTether/
├── crates/
│   ├── engine/                        # Rust engine → AdbTether_core.dll (FFI)
│   │   ├── Cargo.toml                 # package "adbtether-engine"
│   │   └── src/
│   │       ├── lib.rs                 # FFI exports (top_init, top_toggle_tunnel, top_cleanup, …)
│   │       ├── config.rs              # Configuration
│   │       ├── proxy.rs               # Proxy health check
│   │       └── tunnel/
│   │           ├── mod.rs             # Tunnel lifecycle, route management
│   │           ├── ws.rs              # WebSocket (TCP CONNECT + UDP associate)
│   │           ├── udp.rs             # UDP relay (flow-id table)
│   │           └── device.rs          # smoltcp Device implementation
│   └── ws-proxy/                      # ws-proxy (Rust binary pushed to the phone)
│       ├── Cargo.toml                 # package "adbtether-server", binary "ws-proxy"
│       └── src/main.rs                # WebSocket server + TCP/UDP proxy
│
├── src/
│   ├── AdbTetherService/                 # Windows service (C#, SYSTEM)
│   │   ├── EngineService.cs           # ServiceBase lifecycle
│   │   ├── EngineBridge.cs            # FFI P/Invoke → AdbTether_core.dll
│   │   ├── TcpServer.cs               # RPC endpoint (127.0.0.1:47012)
│   │   ├── Program.cs                 # Debug console mode
│   │   └── install.ps1                # Legacy service install (retrofit only)
│   │
│   └── AdbTether/                        # WinUI 3 desktop app (C#)
│       ├── MainWindow.xaml/.cs        # Shell window, title bar, navigation
│       ├── App.xaml/.cs               # Application entry point
│       ├── AdbConnectDialog.xaml/.cs  # TCP/IP connect dialog
│       ├── AdbPairDialog.xaml/.cs     # Pairing dialog
│       ├── Abstractions/              # Interfaces (ICoreService, IAdbService, IDialogService, …)
│       ├── Models/                    # Records/enums (AppInfo, UpdateInfo, Requests, TunnelState)
│       ├── Composition/AppServices.cs # Composition root (hand-rolled DI)
│       ├── Presentation/StatusPresenter.cs  # Status colour/brush mapping
│       ├── Controls/AreaChart.cs      # Traffic area chart
│       ├── Pages/                     # Home, Log, Settings, About
│       ├── Services/                  # AdbService, RemoteCoreService, TunnelConnector,
│       │                              # DialogService, ClipboardService, AppearanceService,
│       │                              # UpdateService, NetworkEndpointHelper
│       ├── ViewModels/                # Main + Status/Traffic/Device/Settings/Update/Log,
│       │                              # RelayCommand, AsyncRelayCommand, ViewModelBase, Dialogs/
│       └── Views/                     # ThemedContentDialog, UpdateDialog
│
├── installer/                         # Packaging
│   ├── Product.wxs                    # MSI layout + ServiceInstall/ServiceControl
│   ├── gen_fragments.ps1              # Recursive payload harvest
│   ├── AppFiles.wxs                   # UI payload (generated)
│   ├── ServiceFiles.wxs               # Service payload (generated)
│   └── dist/AdbTether.msi                # Output (~10 MB)
│
├── assets/                            # Icons and logos
├── docs/                              # Documentation (this file)
├── tests/                             # Integration tests
├── build.ps1                          # One-command build
├── test.ps1                           # Test runner
└── README.md
```

---

## 5. Naming

| Layer | Value | Scope |
|---|---|---|
| **Display name** | **AdbTether** | Window title, About, ARP, service DisplayName, MSI |
| **Code name** | AdbTether | Namespace, assembly, exe names, service key, registry keys |
| **Adapter name** | `AdbTether` | Network adapters, netsh, wintun |
| **ws-proxy binary** | `ws-proxy` | Phone side |

> The MSI UpgradeCode is stable across releases, so a newer version upgrades the existing install in place.

---

## 6. Technology stack

### Engine (Rust)

| Crate | Version | Purpose |
|---|---|---|
| tokio | 1.x | Async runtime |
| smoltcp | 0.13 | User-space TCP/IP stack |
| wintun | 0.5 | Windows TUN adapter binding |
| tokio-tungstenite | 0.26 | WebSocket client |
| serde / serde_json | — | Serialization |
| anyhow | — | Error handling |
| once_cell | — | Static globals |
| rand | — | Random number generation |
| futures-util | — | Async helpers |

### Windows service (C#)

| Package | Purpose |
|---|---|
| .NET 10.0-windows | Target framework |
| System.ServiceProcess.ServiceController | Service management |
| System.Diagnostics.EventLog | Event Viewer logging |

### UI (WinUI 3 / C#)

| Package | Purpose |
|---|---|
| Microsoft.WindowsAppSDK 1.8 | WinUI 3 runtime |
| AdvancedSharpAdbClient 3.6.16 | ADB device management |

### Packaging

| Tool | Version | Purpose |
|---|---|---|
| WiX Toolset | 5.0 | MSI creation |

---

## 7. RPC API

The service exposes JSON over TCP on 127.0.0.1:47012. Each request is one JSON line; each reply is one JSON object.

| Op | Parameters | Response | Description |
|---|---|---|---|
| `set_device` | `{device}` | ok | Set the ADB device |
| `set_dns_mode` | `{mode}` (0/1) | ok | Change the DNS resolution mode |
| `get_dns_mode` | — | `{value}` | Read the DNS mode |
| `set_port` | `{port}` | ok | Proxy port |
| `get_port` | — | `{value}` | Read the proxy port |
| `set_filter_hosts` | `{hosts}` (comma-separated) | ok | Filter list (live update) |
| `get_filter_hosts` | — | `{value}` | Read the filter list |
| `set_ipv6_block` | `{value}` (0/1) | ok | IPv6 kill-switch |
| `get_ipv6_block` | — | `{value}` | IPv6 state |
| `connect` | — | `{result}` | Start the connection |
| `toggle` | — | `{result}` | Connect/suspend toggle |
| `disconnect` | — | ok | Disconnect (only stops the monitor) |
| `get_status` | — | `{value}` (string) | Current status |
| `is_connected` | — | `{value}` (bool) | Connection state |
| `is_tunnel_active` | — | `{value}` (bool) | Whether the tunnel is active |
| `get_stats` | — | `{value}` (JSON) | Uplink/downlink byte counters |
| `reset_stats` | — | ok | Reset the counters |
| `get_log` | — | `{value}` (JSON array) | New log lines |
| `clear_log` | — | ok | Clear the log buffer |

---

## 8. Leak protection (detail)

### IPv6 kill-switch

- `up()`: a gateway ULA anchor (`fd8a:8a8a:8a8a::2/64`) is assigned to the adapter and a `::/0` blackhole route is added.
- `down()`: the route and the anchor are removed.
- Toggle: enabled/disabled instantly from the IPv6 setting.
- Physical block: the Windows default v6 route (the physical adapter's `::/0`) now prefers the tunnel at metric=1 → traffic is dropped (fail-closed).

### DNS pinning

- Polled in `up()`: `default_physical_gateway()` finds the physical gateway **before our own default route is added**, so a snapshot is taken.
- The adapter DNS is pinned to `1.1.1.1` and `8.8.8.8` (`set dnsservers`).
- Mode 0: DNS queries exit through the tunnel from the phone → the phone's network resolves them.
- Mode 1: DNS queries exit from the physical network via the bypass route → the user's own network DNS.

### Host exclusions (live)

- When `set_filter_hosts` is called, only the **added/removed** hosts are computed (differential).
- Existing routes are left untouched → no connection flapping.
- All bypass routes are cleared in `down()`.

### Flow-ID table

- `handle_incoming()`: each source socket (`app` addr) is assigned a unique ID (0 is skipped on wrap-around).
- Insert: `HashMap<(flow_id, target) → app` + ring eviction (8192 capacity).
- Reply demux: looked up by the `(flow_id, source)` key; `flow_id=0` → legacy map.
- Legacy fallback: compatible with the ws-proxy as of 2026-08-20 (RSV=0 echo).

---

## 9. Packaging (MSI)

### Why MSI?

- **Windows Installer:** a trusted platform; ML AV heuristics are neutral toward MSI.
- **Declarative service:** `ServiceInstall`/`ServiceControl` tables (SCM API directly, no scripts).
- **Zero script surface:** no `sc.exe`, `PowerShell.exe`, `taskkill` or `.ps1` files.
- **Silent install:** `msiexec /i /qn` natively.
- **Upgrade:** `MajorUpgrade` removes the previous version automatically.

### Why was NSIS removed?

NSIS is heavily abused by malware, so ML models kept flagging it as "Generic ML PUA". Behaviour cleanup (SCM API, raw sockets) helped somewhat, but the packager itself was still flagged by ML. Moving to MSI solved the problem at the root.

---

## 10. Build and distribution

### Development

```bash
# Engine (Rust)
cargo build --release -p adbtether-engine

# Service (C#)
dotnet build src/AdbTetherService

# UI (C# / WinUI 3)
dotnet build src/AdbTether

# ws-proxy (Rust → Android ARM64)
cargo build --release --target aarch64-linux-android -p adbtether-server
# (requires the NDK + rustup target; PATH must be set correctly)
```

Or, from the project root, the one-command build:

```powershell
./build.ps1                 # Rust engine + ws-proxy + service + UI + MSI
./build.ps1 -SkipMsi        # everything except the MSI
```

### Building the MSI

```bash
# Generate the fragments (when the payload changes)
powershell -File installer/gen_fragments.ps1 -Source <UI_BIN>  -VarName UI  -Group AppFilesGroup    -DirRef APPDIR     -OutFile installer/AppFiles.wxs    -Prefix AF_
powershell -File installer/gen_fragments.ps1 -Source <SVC_BIN> -VarName SVC -Group ServiceFilesGroup -DirRef SERVICEDIR -OutFile installer/ServiceFiles.wxs -Prefix SV_ -Service

# Build the MSI
wix build -arch x64 -d UI=<UI_BIN> -d SVC=<SVC_BIN> -o installer/dist/AdbTether.msi installer/Product.wxs installer/AppFiles.wxs installer/ServiceFiles.wxs
```

### Installation

```bash
msiexec /i AdbTether.msi        # GUI install
msiexec /i AdbTether.msi /qn    # silent install
msiexec /x AdbTether.msi /qn    # silent uninstall
```

### Legacy service install (`src/AdbTetherService`)

```powershell
# Legacy path (unnecessary while the MSI exists; only for retrofit / testing)
powershell -File src/AdbTetherService/install.ps1
powershell -File src/AdbTetherService/install.ps1 -Uninstall
```

---

## 11. Known issues

| Issue | Status | Description |
|---|---|---|
| XamlCompiler.exe flakiness | Expected | The WinUI 1.8 XamlCompiler + .NET 10 MSBuild integration can crash silently and affect `dotnet build`. CI works with `UseXamlCompilerExecutable=true`; locally, prefer a graceful API over `taskkill /F`. |
| `AdbTether_core.dll` stale | Mitigated | Because of a global `cargo target-dir`, csproj paths can point at a stale DLL; the DLL is copied after a rebuild. |
| AdbTetherService leftover | Inventory note | A leftover service from the old name cannot be removed with `net stop`; it must be cleared by reinstalling. |
| ws-proxy binary mirror | Manual rebuild | The ws-proxy pushed to Android is mirrored under `crates/ws-proxy/target`; the published copy is not used. |
| WebSocket fragmentation | Low | The binary ws-proxy may send fragmented WS frames and `ClientWebSocket` can deliver them in pieces — resolved in the PowerShell test with an accumulator loop. |

---

## 12. Roadmap

| Priority | Suggestion |
|---|---|
| **Connection reliability** | Automatic reconnect when the proxy restarts; exponential backoff |
| **Cross-platform** | A tungstenite-based peer for macOS/Linux; wireguard-go tun instead of wintun |
| **Enable DNS_MODE=1** | May stay a hidden UI toggle, but must genuinely work at runtime |
| **Trusted Signing** | Code signing via Azure Trusted Signing (~$10/month) |
| **top_cleanup ghost-delete guard** | Guard against broken system entries when the virtual gateway is empty |
