using System.Net.WebSockets;
using AdbTether.Abstractions;

namespace AdbTether.Services;

/// <summary>
/// Brings up the on-device WebSocket proxy over adb: pushes the binary,
/// (re)starts it, creates the adb forward, then waits until it accepts
/// connections. Remembers success so a reconnect can skip straight to resume.
/// </summary>
public sealed class TunnelConnector
{
    private static readonly TimeSpan ProxyTimeout = TimeSpan.FromSeconds(12);
    private static readonly TimeSpan ProbeTimeout = TimeSpan.FromMilliseconds(800);
    private static readonly TimeSpan RetryDelay = TimeSpan.FromMilliseconds(400);

    private const string RemotePath = "/data/local/tmp/ws-proxy";

    private readonly IAdbService _adb;
    private bool _proxyReady;

    public TunnelConnector(IAdbService adb) => _adb = adb;

    /// <summary>True once the on-device proxy has answered on the forwarded port.</summary>
    public bool IsProxyReady => _proxyReady;

    /// <summary>
    /// Ensures the on-device proxy is running and reachable on <paramref name="port"/>.
    /// Returns false if it never came up; throws if the adb steps themselves fail.
    /// </summary>
    public async Task<bool> EnsureProxyAsync(
        string device,
        int port,
        IProgress<string>? report = null,
        CancellationToken ct = default)
    {
        report?.Report("Pushing ws-proxy to device...");
        string localWsProxy = Path.Combine(AppContext.BaseDirectory, "ws-proxy");
        if (File.Exists(localWsProxy))
            await _adb.PushFileAsync(device, localWsProxy, RemotePath, ct);
        else
            report?.Report("ws-proxy binary not found next to app; assuming it is already on device.");

        await _adb.KillProxyAsync(device, ct);
        await _adb.CreateForwardAsync(device, port, port, ct);
        await _adb.StartProxyAsync(device, port, ct: ct);

        report?.Report("Waiting for proxy...");
        _proxyReady = await WaitForProxyAsync(port, report, ct);
        return _proxyReady;
    }

    /// <summary>Stops the on-device proxy (best effort).</summary>
    public async Task KillProxyAsync(string device, CancellationToken ct = default)
    {
        await _adb.KillProxyAsync(device, ct);
        _proxyReady = false;
    }

    /// <summary>
    /// Marks the on-device proxy as no longer ready, so the next connect runs
    /// the push/forward/start sequence again instead of skipping straight to a
    /// resume. Called when the engine reports the proxy connection was lost.
    /// </summary>
    public void MarkProxyUnreachable() => _proxyReady = false;

    private static async Task<bool> WaitForProxyAsync(int port, IProgress<string>? report, CancellationToken ct)
    {
        using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
        cts.CancelAfter(ProxyTimeout);
        int attempt = 0;

        while (!cts.IsCancellationRequested)
        {
            attempt++;
            string lastError = "unknown";
            try
            {
                using var ws = new ClientWebSocket();
                using var linked = CancellationTokenSource.CreateLinkedTokenSource(cts.Token);
                linked.CancelAfter(ProbeTimeout);
                await ws.ConnectAsync(new Uri($"ws://127.0.0.1:{port}/"), linked.Token);
                if (ws.State == WebSocketState.Open)
                    return true;
                lastError = $"state={ws.State}";
            }
            catch (Exception ex)
            {
                lastError = ex.InnerException?.Message ?? ex.Message;
                if (lastError.Contains("operation was canceled") || lastError.Contains("The operation"))
                    lastError = "timed out";
            }

            report?.Report($"Waiting for proxy... attempt {attempt}: {lastError}");
            try
            {
                await Task.Delay(RetryDelay, cts.Token);
            }
            catch (OperationCanceledException)
            {
                break;
            }
        }

        return false;
    }
}
