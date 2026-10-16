using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using AdbTether.Abstractions;

namespace AdbTether.Services;

/// <summary>
/// RPC client for AdbTetherService. Commands are forwarded over loopback TCP to the
/// SYSTEM service, which performs the admin work (wintun adapter, routes, netsh)
/// without requiring the UI process to be elevated.
/// </summary>
public sealed class RemoteCoreService : ICoreService
{
    private const string Host = "127.0.0.1";
    private const int Port = 47012;
    private const string OfflineMessage = "AdbTetherService not running (start it or run install.ps1 as admin).";

    // A connect to a closed loopback port does not fail instantly on Windows; it
    // burns ~2 s in SYN retries. Bound it, and once the service is known to be
    // down stop re-probing on every call: the UI polls synchronously, so an
    // unbounded connect would keep the UI thread permanently blocked.
    private const int ConnectTimeoutMs = 1000;
    private const int OfflineCooldownMs = 5000;

    private TcpClient? _client;
    private StreamReader? _reader;
    private StreamWriter? _writer;
    private bool _available;
    private string _lastError = "";
    private DateTime _retryAfterUtc = DateTime.MinValue;

    public bool IsAvailable => _available;

    public string LastError => _lastError;

    public void Init()
    {
        try
        {
            Open();
            _available = true;
            _lastError = "";
            _retryAfterUtc = DateTime.MinValue;
        }
        catch (Exception ex)
        {
            _available = false;
            _lastError = ex.Message;
            _retryAfterUtc = DateTime.UtcNow.AddMilliseconds(OfflineCooldownMs);
        }
    }

    private void Open()
    {
        if (_client is { Connected: true })
            return;

        var client = new TcpClient();
        try
        {
            // Never block the caller indefinitely: a missing service (or a
            // firewall that drops the SYN) must not freeze the UI thread.
            var connect = client.ConnectAsync(Host, Port);
            if (!connect.Wait(ConnectTimeoutMs))
            {
                _ = connect.ContinueWith(static t => _ = t.Exception, TaskContinuationOptions.OnlyOnFaulted);
                throw new TimeoutException(
                    $"AdbTetherService did not answer on {Host}:{Port} within {ConnectTimeoutMs} ms.");
            }
        }
        catch
        {
            client.Dispose();
            throw;
        }

        var stream = client.GetStream();
        _client = client;
        _reader = new StreamReader(stream, Encoding.UTF8, false, 1024, leaveOpen: true);
        _writer = new StreamWriter(stream, Encoding.UTF8, 1024, leaveOpen: true) { AutoFlush = true };
    }

    private void Close()
    {
        _writer?.Dispose();
        _reader?.Dispose();
        _client?.Dispose();
        _writer = null;
        _reader = null;
        _client = null;
    }

    private JsonElement Send(string op, params (string Key, object? Value)[] args)
    {
        string request = BuildRequest(op, args);

        // Fast-fail while the service is known to be down: retrying on every poll
        // tick would keep the UI thread blocked on the connect timeout.
        if (!_available && DateTime.UtcNow < _retryAfterUtc)
            throw new InvalidOperationException($"AdbTetherService is not running: {_lastError}");

        try
        {
            EnsureOpen();
            return ReadResponse(request);
        }
        catch (Exception first)
        {
            // Already known to be down: do not pay for a second connect attempt.
            if (!_available)
            {
                GoOffline(first);
                throw new InvalidOperationException($"AdbTetherService is not running: {_lastError}");
            }

            // One retry: reopen the connection and try once more.
            try
            {
                Close();
                Open();
                _available = true;
                _retryAfterUtc = DateTime.MinValue;
                return ReadResponse(request);
            }
            catch (Exception ex)
            {
                GoOffline(ex);
                throw new InvalidOperationException($"AdbTetherService is not running: {_lastError}");
            }
        }
    }

    private void GoOffline(Exception ex)
    {
        _available = false;
        _lastError = ex.Message;
        _retryAfterUtc = DateTime.UtcNow.AddMilliseconds(OfflineCooldownMs);
    }

    private static string BuildRequest(string op, (string Key, object? Value)[] args)
    {
        var body = new Dictionary<string, object?> { ["op"] = op };
        foreach (var (key, value) in args)
            body[key] = value;
        return JsonSerializer.Serialize(body);
    }

    private void EnsureOpen()
    {
        if (_client is not { Connected: true })
        {
            Close();
            Open();
        }
    }

    private JsonElement ReadResponse(string request)
    {
        _writer!.WriteLine(request);
        string? response = _reader!.ReadLine()
            ?? throw new IOException("service closed the connection");

        using var doc = JsonDocument.Parse(response);
        var root = doc.RootElement;
        if (root.TryGetProperty("error", out var err))
            throw new InvalidOperationException(err.GetString());
        if (root.TryGetProperty("value", out var value))
            return value.Clone();
        if (root.TryGetProperty("result", out var result))
            return result.Clone();
        return root.Clone();
    }

    // ── engine commands ──────────────────────────────────────────────────

    public void SetDevice(string device) => Safe(() => Send("set_device", ("device", device)));

    public void SetDnsMode(int mode) => Safe(() => Send("set_dns_mode", ("mode", mode)));

    public int GetIpv6Block() => SafeInt(() => Send("get_ipv6_block").GetInt32());

    public void SetIpv6Block(int value) => Safe(() => Send("set_ipv6_block", ("value", value)));

    public ushort GetPort() => (ushort)SafeInt(() => Send("get_port").GetInt32());

    public void SetPort(ushort port) => Safe(() => Send("set_port", ("port", (int)port)));

    public void SetFilterHosts(string hosts) => Safe(() => Send("set_filter_hosts", ("hosts", hosts)));

    public string GetFilterHosts() => SafeString(() => Send("get_filter_hosts").GetString());

    public string GetStatus() => SafeString(() => Send("get_status").GetString());

    public bool IsTunnelActive() => SafeBool(() => Send("is_tunnel_active").GetBoolean());

    public bool IsConnected() => SafeBool(() => Send("is_connected").GetBoolean());

    public int ToggleTunnel() => SafeInt(() => Send("toggle").GetInt32());

    public void ResetStats() => Safe(() => Send("reset_stats"));

    public (long UplinkBytes, long DownlinkBytes) GetStats()
    {
        try
        {
            string raw = Send("get_stats").GetString() ?? "";
            using var doc = JsonDocument.Parse(raw);
            var root = doc.RootElement;
            long up = root.TryGetProperty("uplink_bytes", out var u) ? u.GetInt64() : 0;
            long down = root.TryGetProperty("downlink_bytes", out var d) ? d.GetInt64() : 0;
            return (up, down);
        }
        catch
        {
            return (0, 0);
        }
    }

    public IReadOnlyList<string> GetLog()
    {
        try
        {
            string raw = Send("get_log").GetString() ?? "";
            if (string.IsNullOrEmpty(raw))
                return Array.Empty<string>();
            return JsonSerializer.Deserialize<string[]>(raw) ?? Array.Empty<string>();
        }
        catch
        {
            return Array.Empty<string>();
        }
    }

    public void ClearLog() => Safe(() => Send("clear_log"));

    // ── safe wrappers: never throw when the service is offline ───────────

    private void Safe(Action action)
    {
        try { action(); } catch { MarkOffline(); }
    }

    private int SafeInt(Func<int> func)
    {
        try { return func(); } catch { MarkOffline(); return 0; }
    }

    private bool SafeBool(Func<bool> func)
    {
        try { return func(); } catch { MarkOffline(); return false; }
    }

    private string SafeString(Func<string?> func)
    {
        try { return func() ?? ""; } catch { MarkOffline(); return ""; }
    }

    private void MarkOffline()
    {
        // Send() has already recorded the specific error and started the cooldown;
        // this only guarantees there is always a message to show. It must not
        // extend the cooldown, or the poll would never let it expire.
        _available = false;
        if (string.IsNullOrEmpty(_lastError))
            _lastError = OfflineMessage;
    }
}
