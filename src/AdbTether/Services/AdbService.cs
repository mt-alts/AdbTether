using AdvancedSharpAdbClient;
using AdvancedSharpAdbClient.Models;
using System.Diagnostics;
using System.IO;
using AdbTether.Abstractions;
using AdbTether.Models;

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
    private readonly IAdbServer _server = AdbServer.Instance;
    private readonly SemaphoreSlim _serverGate = new(1, 1);
    private readonly Dictionary<string, string> _names = new(StringComparer.Ordinal);

    public async Task<IReadOnlyList<AdbDevice>> GetDevicesAsync(CancellationToken ct = default)
    {
        await EnsureServerAsync(ct);
        var devices = await _client.GetDevicesAsync(ct);
        var online = devices
            .Where(d => d.State == DeviceState.Online)
            .Select(d => d.Serial)
            .ToArray();

        var result = new List<AdbDevice>(online.Length);
        foreach (string serial in online)
        {
            if (!_names.TryGetValue(serial, out string? name))
            {
                name = await ResolveNameAsync(serial, ct);
                _names[serial] = name;
            }
            result.Add(new AdbDevice(serial, name));
        }
        return result;
    }

    public async Task<string> ConnectAsync(string host, int port, CancellationToken ct = default)
    {
        await EnsureServerAsync(ct);
        var result = await _client.ConnectAsync(host, port, ct);
        return result?.ToString() ?? "";
    }

    public async Task<string> PairAsync(string host, int port, string code, CancellationToken ct = default)
    {
        await EnsureServerAsync(ct);
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
        await EnsureServerAsync(ct);
        var devices = await _client.GetDevicesAsync(ct);
        return devices.FirstOrDefault(d => d.Serial == serial && d.State == DeviceState.Online);
    }

    /// <summary>
    /// The adb client only talks to a server that is already listening, so on a
    /// PC where adb.exe is installed but not running every scan comes back empty
    /// and refreshing does not help. Bring the server up before the first call.
    /// </summary>
    private async Task EnsureServerAsync(CancellationToken ct)
    {
        if ((await _server.GetStatusAsync(ct)).IsRunning)
            return;

        // One start at a time: the shell refresh and a connect can overlap.
        await _serverGate.WaitAsync(ct);
        try
        {
            if ((await _server.GetStatusAsync(ct)).IsRunning)
                return;

            await _server.StartServerAsync(AdbPath, restartServerIfNewer: false, ct);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            throw new InvalidOperationException(
                $"adb server could not be started from '{AdbPath}'. Install Android " +
                $"platform-tools or put adb.exe on PATH. ({ex.Message})", ex);
        }
        finally
        {
            _serverGate.Release();
        }
    }

    /// <summary>
    /// A serial means nothing to a human, so ask the phone what it calls itself.
    /// ro.product.marketname ("Xiaomi 13T") is the name the vendor sells it
    /// under; older devices only answer manufacturer + model ("Samsung
    /// SM-S911B"), and a phone that answers nothing keeps its serial. One call
    /// per device, cached for the session.
    /// </summary>
    private static async Task<string> ResolveNameAsync(string serial, CancellationToken ct)
    {
        string output = await RunCliAsync(
            $"-s {serial} shell \"getprop ro.product.marketname; " +
            "getprop ro.product.vendor.marketname; " +
            "getprop ro.product.manufacturer; getprop ro.product.model\"", ct);

        if (output.StartsWith("exit ", StringComparison.Ordinal))
            return serial;

        string[] lines = output.Replace("\r", "").Split('\n');
        string At(int index) => index < lines.Length ? lines[index].Trim() : "";

        string market = At(0).Length > 0 ? At(0) : At(1);
        if (market.Length > 0)
            return market;

        string fallback = string.Join(' ', new[] { At(2), At(3) }.Where(p => p.Length > 0));
        return fallback.Length > 0 ? fallback : serial;
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
