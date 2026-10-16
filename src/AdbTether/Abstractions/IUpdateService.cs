using AdbTether.Models;

namespace AdbTether.Abstractions;

/// <summary>Checks for and installs application updates.</summary>
public interface IUpdateService
{
    Task<UpdateInfo?> CheckForUpdateAsync(CancellationToken ct = default);

    Task DownloadAndInstallAsync(
        UpdateInfo info,
        IProgress<double>? progress = null,
        CancellationToken ct = default);
}
