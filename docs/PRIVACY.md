# Privacy Policy

AdbTether is a local desktop tool. It has no account system, no telemetry, no
analytics and no advertising, and it does not collect or transmit personal data.

This policy describes every place where AdbTether touches data, on your machine
and on the network. It covers the desktop app (`AdbTether.exe`), the Windows
service (`AdbTetherService.exe`) and the phone-side proxy (`ws-proxy`).

Last updated: 2026-10-17.

## Summary

| Question | Answer |
|---|---|
| Do you collect personal data? | No. |
| Is there telemetry or analytics? | No. |
| Is there crash reporting? | No - a crash is written to a local file only. |
| Are there third-party SDKs or ads? | No. |
| Do you sell or share data? | No - there is nothing to share. |
| Does the app need an account? | No. |
| What does the app send over the internet? | One version check against the public GitHub releases API, and the installer download if you accept an update. |

## Network communication

AdbTether makes exactly one outbound request on its own:

- **Update check.** When you press *Check for updates*, or when the startup
  check runs, the app sends `GET`
  `https://api.github.com/repos/mt-alts/AdbTether/releases/latest` with the
  user agent `AdbTether`. The response carries the latest release tag and its
  release notes.
- **Update download.** If you accept an update, the installer (`.msi`) is
  downloaded from the release asset URL on GitHub into `%TEMP%\AdbTether\` and
  then started with `msiexec`.

No other request is made, and nothing is sent to the maintainer.

GitHub sees your IP address and the user agent as part of those requests; that
handling is covered by
[GitHub's privacy statement](https://docs.github.com/site-policy/privacy-policies/github-privacy-statement).
If you would rather not make the request at all, turn the startup check off in
*Settings* and do not use the in-app updater - releases can always be
downloaded by hand from the
[releases page](https://github.com/mt-alts/AdbTether/releases).

### Your internet traffic

While the tunnel is up, the PC's traffic is routed through the phone. AdbTether
forwards packets; it does not inspect, decrypt, filter, store or log their
contents. TLS sessions between your apps and their servers pass through
untouched, and AdbTether cannot read them.

The tunnel itself is a WebSocket connection that reaches the phone over
`adb forward` on the PC's loopback interface. It is not a remote service: no
third party sees your traffic that would not already see it on the phone's own
connection.

## Local data

Everything below stays on your machine.

| Path | Contents | Written by |
|---|---|---|
| `AdbTether.appearance.json` (install folder) | The chosen theme (`system`, `light` or `dark`). | App |
| `%LOCALAPPDATA%\AdbTether\update-preferences.json` | A single boolean: whether the startup update check is skipped. | App |
| `%TEMP%\AdbTether_service.log` | Service startup lines, used to debug a service installation. | Service |
| `crash.log` (install folder) | The text of an unhandled UI exception, if one occurs. | App |
| `%TEMP%\AdbTether\*.msi` | A downloaded installer, kept until the update runs. | App |
| `HKCU\Software\AdbTether` | Two integer values (`sm`, `desk`) that mark the Start-menu and desktop shortcuts as installed. | Installer |

None of these files holds personal data, and none of them is ever uploaded.

### On the phone

Nothing is installed on the Android device, and no Android permission beyond the
ADB link is used. The `ws-proxy` binary is pushed to `/data/local/tmp/ws-proxy`,
run from there, and its output goes to `/data/local/tmp/ws-proxy.log`. The
process is killed when the tunnel is disconnected. Device serials are read
locally through ADB so the device list can be shown; they are not transmitted
anywhere.

## Local service endpoint

The service exposes its RPC endpoint on `127.0.0.1:47012`, on the loopback
interface only. It is not reachable from the network.

## Uninstalling and removing leftovers

`msiexec /x AdbTether.msi` removes the program, the service and the shortcuts.
The items below are not removed by the uninstaller and can be deleted by hand:

- `%LOCALAPPDATA%\AdbTether\` (update preferences)
- `%TEMP%\AdbTether\` (downloaded installers)
- `%TEMP%\AdbTether_service.log`
- `HKCU\Software\AdbTether`
- `/data/local/tmp/ws-proxy` and `/data/local/tmp/ws-proxy.log` on the phone

## Downloads from a mirror

If you get AdbTether from a third-party host such as SourceForge, that host
keeps its own access logs and its own privacy policy applies to them. The
project publishes its own policy here.

## Changes

Any change to how AdbTether handles data is made in the source tree and noted in
this file, so its history is visible in the repository log.

## Contact

Questions or corrections: open an issue at
https://github.com/mt-alts/AdbTether/issues.
