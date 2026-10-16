using System.Runtime.InteropServices;
using System.Text;

namespace AdbTetherService;

/// <summary>
/// Thin FFI wrapper over AdbTether_core.dll. The service runs as SYSTEM so the
/// wintun adapter + route/netsh work performed inside the engine has admin
/// privileges without any UAC prompt in the UI.
/// </summary>
internal static class EngineBridge
{
    private const string Dll = "AdbTether_core.dll";

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_init();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_set_device([MarshalAs(UnmanagedType.LPUTF8Str)] string device);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_set_dns_mode(int mode);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_get_dns_mode();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_set_ipv6_block(int v);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_get_ipv6_block();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_set_port(ushort port);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern ushort top_get_port();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_set_filter_hosts([MarshalAs(UnmanagedType.LPUTF8Str)] string hosts);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_get_filter_hosts([Out, MarshalAs(UnmanagedType.LPUTF8Str)] StringBuilder buf, int bufLen);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_connect();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_toggle_tunnel();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_disconnect();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_get_status([Out, MarshalAs(UnmanagedType.LPUTF8Str)] StringBuilder buf, int bufLen);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_get_stats([Out, MarshalAs(UnmanagedType.LPUTF8Str)] StringBuilder buf, int bufLen);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_reset_stats();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_get_log([Out, MarshalAs(UnmanagedType.LPUTF8Str)] StringBuilder buf, int bufLen);

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_clear_log();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_tunnel_active();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern int top_is_connected();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_cleanup();

    [DllImport(Dll, CallingConvention = CallingConvention.Cdecl)]
    private static extern void top_teardown();

    public static int Init() => top_init();

    public static int Connect() => top_connect();

    public static int ToggleTunnel() => top_toggle_tunnel();

    public static void Disconnect() => top_disconnect();

    public static bool IsTunnelActive() => top_tunnel_active() == 1;

    public static bool IsConnected() => top_is_connected() == 1;

    public static void Cleanup() => top_cleanup();

    /// <summary>
    /// Leaves the tunnel completely while keeping the adapter: routes, pinned
    /// DNS and the static adapter IP are removed. Called when the UI app goes
    /// away, so closing it never strands the PC on a dead tunnel.
    /// </summary>
    public static void Teardown() => top_teardown();

    public static void SetDevice(string device) => top_set_device(device);

    public static void SetDnsMode(int mode) => top_set_dns_mode(mode);

    public static int GetDnsMode() => top_get_dns_mode();

    public static void SetIpv6Block(int v) => top_set_ipv6_block(v);

    public static int GetIpv6Block() => top_get_ipv6_block();

    public static void SetPort(ushort port) => top_set_port(port);

    public static ushort GetPort() => top_get_port();

    public static void SetFilterHosts(string hosts) => top_set_filter_hosts(hosts);

    public static string GetFilterHosts()
    {
        var buf = new StringBuilder(4096);
        top_get_filter_hosts(buf, buf.Capacity);
        return buf.ToString();
    }

    public static string GetStatus()
    {
        var buf = new StringBuilder(512);
        top_get_status(buf, buf.Capacity);
        return buf.ToString();
    }

    public static string GetStats()
    {
        var buf = new StringBuilder(128);
        top_get_stats(buf, buf.Capacity);
        return buf.ToString();
    }

    public static void ResetStats() => top_reset_stats();

    public static string GetLog()
    {
        var buf = new StringBuilder(16384);
        top_get_log(buf, buf.Capacity);
        return buf.ToString();
    }

    public static void ClearLog() => top_clear_log();
}
