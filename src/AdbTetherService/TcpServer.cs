using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;

namespace AdbTetherService;

/// <summary>
/// Loopback RPC server. The UI (AdbTether) connects to 127.0.0.1:port and
/// sends one-line JSON commands:
///   {"op":"set_device","device":"..."}
///   {"op":"connect"} etc.
/// Each command is answered with a single JSON object on the same connection.
/// Bound to loopback only so it is unreachable from the network; the service
/// runs as SYSTEM so the engine can perform admin work on request.
/// </summary>
internal sealed class TcpServer
{
    private const int Port = 47012;

    /// <summary>
    /// Grace period after the last RPC client disconnects before the tunnel is
    /// torn down. Absorbs the UI client's close-and-reopen retry window and a
    /// quick app restart, so a transient blip never kills a live tunnel.
    /// </summary>
    private static readonly TimeSpan IdleTeardownDelay = TimeSpan.FromSeconds(3);

    private readonly CancellationToken _ct;
    private readonly Action<string> _trace;

    // Live client count + a generation token. The token is bumped whenever a
    // client connects (cancelling a pending teardown) or a new idle timer is
    // armed, so a stale timer can never fire against a newer state.
    private readonly object _clientsGate = new();
    private int _clients;
    private int _generation;

    public TcpServer(CancellationToken ct, Action<string> trace)
    {
        _ct = ct;
        _trace = trace;
    }

    public void Run()
    {
        var listener = new TcpListener(IPAddress.Loopback, Port);
        listener.Start();
        _trace($"[rpc] listening on 127.0.0.1:{Port}");
        try
        {
            while (!_ct.IsCancellationRequested)
            {
                TcpClient client;
                try
                {
                    client = listener.AcceptTcpClient();
                }
                catch
                {
                    break;
                }
                _trace("[rpc] client connected");
                OnClientConnected();
                ThreadPool.QueueUserWorkItem(_ => HandleClient(client));
            }
        }
        finally
        {
            listener.Stop();
        }
    }

    private void HandleClient(TcpClient client)
    {
        try
        {
            using (client)
            using (var net = client.GetStream())
            using (var reader = new StreamReader(net, Encoding.UTF8, false, 1024, leaveOpen: true))
            using (var writer = new StreamWriter(net, Encoding.UTF8, 1024, leaveOpen: true) { AutoFlush = true })
            {
                while (!_ct.IsCancellationRequested)
                {
                    string? line;
                    try
                    {
                        line = reader.ReadLine();
                    }
                    catch
                    {
                        return; // client disconnected
                    }
                    if (line is null)
                        return;
                    _trace($"[rpc] recv: {line}");

                    string response;
                    try
                    {
                        response = Dispatch(line);
                    }
                    catch (Exception ex)
                    {
                        response = Json("error", ex.Message);
                    }
                    _trace($"[rpc] resp: {response}");
                    try
                    {
                        writer.WriteLine(response);
                    }
                    catch
                    {
                        return;
                    }
                }
            }
        }
        finally
        {
            OnClientDisconnected();
        }
    }

    /// <summary>
    /// A new client is attached: bump the generation so any pending idle
    /// teardown is abandoned, then count it.
    /// </summary>
    private void OnClientConnected()
    {
        lock (_clientsGate)
        {
            _clients++;
            _generation++;
        }
    }

    /// <summary>
    /// A client left. When the last one goes, arm a delayed teardown of the
    /// tunnel: the UI closing is only detectable as "the RPC socket closed", and
    /// leaving the PC routed into a tunnel whose app is gone would strand it
    /// without connectivity. The delay + generation guard mean a quick
    /// reconnect or restart keeps the tunnel alive.
    /// </summary>
    private void OnClientDisconnected()
    {
        int generation;
        lock (_clientsGate)
        {
            if (--_clients > 0)
                return;
            generation = ++_generation;
        }

        _ = Task.Run(async () =>
        {
            try
            {
                await Task.Delay(IdleTeardownDelay, _ct).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return; // service stopping; its own cleanup handles the tunnel
            }

            lock (_clientsGate)
            {
                if (_clients != 0 || _generation != generation)
                    return; // a client came back (or newer timer armed)
            }

            _trace($"[rpc] last client gone for {IdleTeardownDelay.TotalSeconds:0}s, tearing tunnel down");
            try
            {
                EngineBridge.Teardown();
            }
            catch (Exception ex)
            {
                _trace($"[rpc] teardown failed: {ex.Message}");
            }
        });
    }

    private static string Dispatch(string line)
    {
        using var doc = JsonDocument.Parse(line);
        var root = doc.RootElement;
        string op = root.GetProperty("op").GetString() ?? "";

        return op switch
        {
            "set_device" => Cmd(() => { EngineBridge.SetDevice(GetString(root, "device")); return Ok(); }),
            "set_dns_mode" => Cmd(() => { EngineBridge.SetDnsMode(root.GetProperty("mode").GetInt32()); return Ok(); }),
            "set_ipv6_block" => Cmd(() => { EngineBridge.SetIpv6Block(root.GetProperty("value").GetInt32()); return Ok(); }),
            "get_ipv6_block" => Cmd(() => Json("value", EngineBridge.GetIpv6Block())),
            "set_port" => Cmd(() => { EngineBridge.SetPort((ushort)root.GetProperty("port").GetInt32()); return Ok(); }),
            "set_filter_hosts" => Cmd(() => { EngineBridge.SetFilterHosts(GetString(root, "hosts")); return Ok(); }),
            "get_filter_hosts" => Cmd(() => Json("value", EngineBridge.GetFilterHosts())),
            "connect" => Cmd(() => Json("result", EngineBridge.Connect())),
            "toggle" => Cmd(() => Json("result", EngineBridge.ToggleTunnel())),
            "disconnect" => Cmd(() => { EngineBridge.Disconnect(); return Ok(); }),
            "get_status" => Cmd(() => Json("value", EngineBridge.GetStatus())),
            "get_stats" => Cmd(() => Json("value", EngineBridge.GetStats())),
            "reset_stats" => Cmd(() => { EngineBridge.ResetStats(); return Ok(); }),
            "get_log" => Cmd(() => Json("value", EngineBridge.GetLog())),
            "clear_log" => Cmd(() => { EngineBridge.ClearLog(); return Ok(); }),
            "is_connected" => Cmd(() => Json("value", EngineBridge.IsConnected())),
            "is_tunnel_active" => Cmd(() => Json("value", EngineBridge.IsTunnelActive())),
            "get_dns_mode" => Cmd(() => Json("value", EngineBridge.GetDnsMode())),
            "get_port" => Cmd(() => Json("value", EngineBridge.GetPort())),
            _ => Json("error", $"unknown op: {op}"),
        };
    }

    private static string Cmd(Func<string> f)
    {
        try
        {
            return f();
        }
        catch (Exception ex)
        {
            return Json("error", ex.Message);
        }
    }

    private static string Ok() => Json("ok", true);

    private static string Json(string key, object value) =>
        JsonSerializer.Serialize(new Dictionary<string, object?> { [key] = value });

    private static string GetString(JsonElement root, string name) =>
        root.TryGetProperty(name, out var v) ? v.GetString() ?? "" : "";
}
