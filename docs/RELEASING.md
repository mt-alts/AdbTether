# Releasing AdbTether

All commands run from the repository root.

## 1. Bump the version

The version is declared in three places and they must stay in sync:

| File | Field | Format |
|---|---|---|
| `src/AdbTether/AdbTether.csproj` | `<Version>` | `0.1.0` |
| `src/AdbTetherService/AdbTetherService.csproj` | `<Version>` | `0.1.0` |
| `installer/Product.wxs` | `Package/@Version` | `0.1.0.0` |

The About page reads the UI assembly version, so the two `csproj` values drive
what the app reports. The MSI `ProductVersion` must be **higher** than the
installed one for the MajorUpgrade to replace it in place.

## 2. Build and test

```powershell
./test.ps1      # Rust unit + integration tests
./build.ps1     # engine, ws-proxy, service, UI and MSI
```

Outputs:

- `installer/dist/AdbTether.msi`
- `installer/dist/AdbTether.wixpdb`

`build.ps1` wipes the previous `bin` payload before compiling, so a removed or
renamed file can never leak from an earlier build into the MSI.

The MSI carries the cross-compiled phone-side `ws-proxy`, so a release must be
built **without** `-SkipAndroid`. If it is missing from the payload, `build.ps1`
refuses to build the MSI (pass `-AllowMissingProxy` only for a throwaway one).

## 3. Smoke test

```powershell
msiexec /i installer\dist\AdbTether.msi
```

Check that the `AdbTetherService` service is installed and running (Automatic
start), the app connects to a device, and a disconnect restores the normal
network.

## 4. Tag

```powershell
git tag v0.1.0
git push origin main --tags
```

## Signing

The MSI is **not code-signed**. For public distribution, sign it with an
OV/EV certificate before publishing:

```powershell
signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /a `
  installer\dist\AdbTether.msi
```
