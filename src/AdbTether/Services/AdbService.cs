using AdvancedSharpAdbClient;
using AdvancedSharpAdbClient.Models;
using System.Diagnostics;
using System.IO;
using AdbTether.Abstractions;

namespace AdbTether.Services;

/// <summary>
/// All adb interaction lives here (AdvancedSharpAdbClient), keeping the Rust
/// engine focused purely on the WebSocket proxy + tunnel orchestration.
/// </summary>
public sealed class AdbService : IAdbService
{
    private static readonly string[] AdbSearchPaths =
    {
        // Next to the running executable
        Path.Combine(AppContext.BaseDirectory, "adb.exe"),
        // Common SDK install locations
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86),
            "Android", "android-sdk", "platform-tools", "adb.exe"),
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Android", "Sdk", "platform-tools", "adb.exe"),
    };

    private static readonly string AdbPath = FindAdb();
    private readonly IAdbClient _client = AdbClient.Instance;

    public async Task<IReadOnlyList<string>> GetDevicesAsync(CancellationToken ct = default)
    {
        var devices = await _client.GetDevicesAsync(ct);
        return devices
            .Where(d => d.State == DeviceState.Online)
            .Select(d => d.Serial)
            .ToArray();
    }

    public async Task<string> ConnectAsync(string host, int port, CancellationToken ct = default)
    {
        var result = await _client.ConnectAsync(host, port, ct);
        return result?.ToString() ?? "";
    }

    public async Task<string> PairAsync(string host, int port, string code, CancellationToken ct = default)
    {
        var result = await _client.PairAsync(host, port, code, ct);
        return result?.ToString() ?? "";
    }

    public async Task CreateForwardAsync(string serial, int localPort, int devicePort, CancellationToken ct = default)
    {
        var device = await FindDeviceAsync(serial, ct)
            ?? throw new InvalidOperationException($"Device not found: {serial}");
        await _client.CreateForwardAsync(device, $"tcp:{localPort}", $"tcp:{devicePort}", true, ct);
    }

    public async Task KillProxyAsync(string serial, CancellationToken ct = default)
    {
        await RunCliAsync($"-s {serial} shell pkill -9 -f ws-proxy", ct);
    }

    public async Task StartProxyAsync(string serial, int port, int idleTimeoutSecs = 60, CancellationToken ct = default)
    {
        await RunCliAsync($"-s {serial} shell chmod 755 /data/local/tmp/ws-proxy", ct);
        // -t: auto-shutdown after N seconds with no client.
        string cmd = $"nohup /data/local/tmp/ws-proxy -p {port} -b 127.0.0.1 -t {idleTimeoutSecs} >/data/local/tmp/ws-proxy.log 2>&1 &";
        await RunCliAsync($"-s {serial} shell \"{cmd}\"", ct);
    }

    public async Task PushFileAsync(string serial, string localPath, string remotePath, CancellationToken ct = default)
    {
        var device = await FindDeviceAsync(serial, ct)
            ?? throw new InvalidOperationException($"Device not found: {serial}");
        using var service = new SyncService(device);
        await Task.Run(() =>
        {
            using var stream = File.OpenRead(localPath);
            service.Push(stream, remotePath, UnixFileStatus.DefaultFileMode, DateTimeOffset.Now, null);
        }, ct);
        // Make the binary executable.
        await _client.ExecuteRemoteCommandAsync($"chmod 755 {remotePath}", device, _ => true, ct);
    }

    private static string FindAdb()
    {
        foreach (string path in AdbSearchPaths)
        {
            if (File.Exists(path))
                return path;
        }
        // Fall back to PATH (might throw if not found).
        return "adb";
    }

    private async Task<DeviceData?> FindDeviceAsync(string serial, CancellationToken ct)
    {
        var devices = await _client.GetDevicesAsync(ct);
        return devices.FirstOrDefault(d => d.Serial == serial && d.State == DeviceState.Online);
    }

    private static async Task<string> RunCliAsync(string arguments, CancellationToken ct = default)
    {
        using var proc = new Process
        {
            StartInfo = new ProcessStartInfo
            {
                FileName = AdbPath,
                Arguments = arguments,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true,
            },
        };
        proc.Start();
        var output = await proc.StandardOutput.ReadToEndAsync(ct);
        var error = await proc.StandardError.ReadToEndAsync(ct);
        await proc.WaitForExitAsync(ct);
        return proc.ExitCode == 0 ? output.Trim() : $"exit {proc.ExitCode}: {error.Trim()}";
    }
}
