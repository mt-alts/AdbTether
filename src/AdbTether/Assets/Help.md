# AdbTether Help

## Getting started

1. Install AdbTether with the MSI. Windows asks for administrator permission because the app installs a background service and a virtual network adapter.
2. **Developer options are hidden by default.** Open **Settings > About phone** and tap **Build number** seven times. Enter your PIN or pattern if asked; Android shows “You are now a developer”.
3. Go back and open **Settings > System > Developer options** (or **Settings > Developer options**). On Samsung the path is **Settings > About phone > Software information > Build number**; on Xiaomi it is **Settings > About phone > OS/MIUI version**.
4. In Developer options, turn on **USB debugging**.
5. Connect the phone with a USB cable. On the phone, accept the **Allow USB debugging** prompt and the RSA fingerprint.
6. Pick the phone in the device list and press **Connect**. When the tunnel is active, the PC uses the phone's internet connection.

### Wireless debugging (Android 11 and newer)

Developer options must be enabled first.

1. Connect the phone and the PC to the same Wi-Fi network.
2. On the phone, open **Developer options > Wireless debugging** and turn it on.
3. Tap **Pair device with pairing code**. Note the IP address, port, and six-digit code.
4. In AdbTether, open the pairing dialog, enter the IP:port and the code, then connect. If the app asks for a connect address, use the IP address and port shown on the Wireless debugging screen.

### TCP/IP connection (adb tcpip)

1. Enable Developer options and **USB debugging**, connect the phone by USB, and accept the authorization prompt.
2. On the PC, run:

   ```powershell
   adb tcpip 5555
   ```

3. Unplug the USB cable.
4. On the PC, run:

   ```powershell
   adb connect <phone-ip>:5555
   ```

5. In AdbTether, choose the device with the IP address and press **Connect**.

You can use your own adb or the copy in the AdbTether app folder.

## Requirements

- Windows 10 version 1809 (build 17763) or newer, or Windows 11. 64-bit only.
- Microsoft **.NET 10 Runtime** (x64). The installer checks for it and stops with a download link if it is missing.
- Microsoft **Windows App Runtime 1.8** for the desktop interface. If the app does not start, install this runtime from Microsoft.
- Administrator permission during installation. The tunnel itself runs in a Windows service.
- Android Platform-Tools is not required; **adb ships with AdbTether**.

## Daily use

The PC's IPv4 TCP and UDP traffic goes through the phone. If the phone is using a VPN, the PC follows that VPN too; no separate VPN client is needed on the PC.

IPv6 is blocked while the tunnel is active to prevent leaks. The connection is IPv4 only.

If the link to the phone drops, the PC stays offline until you disconnect. It does not silently fall back to local Wi-Fi or Ethernet. **Disconnect** removes the routes and DNS settings, and closing the app returns the PC to its normal network.

Nothing is installed as an app on the phone. A small helper runs from a temporary folder while the tunnel is active.

## Troubleshooting

### The service fails to start (error 1053)

The **.NET 10 Runtime** is missing. Install it from Microsoft, then start `AdbTetherService` again or repair the installation. The setup checks for this dependency on a fresh install.

### The installer says my Windows version is unsupported

Update to **AdbTether 0.1.6 or newer**. Earlier builds could misread Windows 10 and 11 as Windows 8.1.

### No Android device appears

Check the cable, turn on **USB debugging**, and accept the authorization prompt on the phone. Try a different USB port or cable. The bundled adb is used automatically; you do not need to install Platform-Tools.

### Connected, but there is no internet

Check that the phone itself can browse. If the phone's VPN or mobile network is down, the PC stays offline by design. Disconnect and connect again after the phone reconnects.

### Another adb server is running

AdbTether uses its own adb copy. It does not stop adb servers started by another SDK or another application.

### The update closed the app

AdbTether closes itself so the installer can replace its files. The update runs unattended with a progress bar and does not require the setup wizard.

## Updates

Check for updates from **Settings**. The app downloads the latest MSI and starts it with administrator permission. The installation is unattended: you see a progress bar, not the wizard.

Before installing, the app stops only its own bundled adb server. adb processes started from other folders are left alone.

## Privacy, logs, and version

AdbTether has no accounts, no telemetry, and no analytics. It writes a local service log to `%TEMP%\AdbTether_service.log` and keeps update preferences under `%LOCALAPPDATA%\AdbTether`.

The **Log** page in the app shows recent messages for troubleshooting. The repository's `PRIVACY.md` describes the files and the single update request in detail.

Press **F1** anywhere in the app to open this page.
