namespace AdbTether.Abstractions;

/// <summary>
/// The engine control surface exposed to the UI. Implemented by
/// <c>RemoteCoreService</c>, which forwards every call over loopback TCP to the
/// elevated AdbTetherService.
/// </summary>
public interface ICoreService
{
    bool IsAvailable { get; }
    string LastError { get; }

    void Init();

    void SetDevice(string device);
    void SetDnsMode(int mode);

    int GetIpv6Block();
    void SetIpv6Block(int value);

    ushort GetPort();
    void SetPort(ushort port);

    void SetFilterHosts(string hosts);
    string GetFilterHosts();

    string GetStatus();
    bool IsTunnelActive();
    bool IsConnected();
    int ToggleTunnel();

    (long UplinkBytes, long DownlinkBytes) GetStats();
    void ResetStats();

    IReadOnlyList<string> GetLog();
    void ClearLog();
}
