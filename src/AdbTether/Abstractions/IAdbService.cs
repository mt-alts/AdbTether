namespace AdbTether.Abstractions;

/// <summary>adb interaction used to reach the Android device (AdvancedSharpAdbClient).</summary>
public interface IAdbService
{
    Task<IReadOnlyList<string>> GetDevicesAsync(CancellationToken ct = default);

    Task<string> ConnectAsync(string host, int port, CancellationToken ct = default);

    Task<string> PairAsync(string host, int port, string code, CancellationToken ct = default);

    Task CreateForwardAsync(string serial, int localPort, int devicePort, CancellationToken ct = default);

    Task KillProxyAsync(string serial, CancellationToken ct = default);

    Task StartProxyAsync(string serial, int port, int idleTimeoutSecs = 60, CancellationToken ct = default);

    Task PushFileAsync(string serial, string localPath, string remotePath, CancellationToken ct = default);
}
