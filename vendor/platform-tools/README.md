# Android SDK Platform-Tools

Bundled so AdbTether can talk to the phone without a separate Platform-Tools
install. `AdbTether.exe` prefers the copy next to itself; see
`AdbService.FindAdb`.

| Item | Value |
|---|---|
| Package | Android SDK Platform-Tools 36.0.2 |
| adb | 1.0.41 |
| Source | https://developer.android.com/tools/releases/platform-tools |
| License | Apache License 2.0 (see `NOTICE.txt`) |

SHA-256:

- `adb.exe`: `56656270DA132F44E9CB4FB86A12BA965635C80423D43DCDD944D9FEC4AB4622`
- `AdbWinApi.dll`: `A00CF631DD12C82561FFCCEFDB1A99A27C527253B04F06CA8FC1BB86BA2148C4`
- `AdbWinUsbApi.dll`: `E4D72D5BA3BF4B027F1B7A3781AAE0B04712C1A52244BEA277FB57DD75A85702`

To update, replace the binaries and `NOTICE.txt` from a newer official
platform-tools package, update this file, then rebuild the MSI. `NOTICE.txt`
comes from Google's package; do not edit it.
