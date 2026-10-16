namespace AdbTether.Models;

/// <summary>Describes an available release discovered on GitHub.</summary>
public sealed record UpdateInfo(
    string LatestVersion,
    string ReleaseUrl,
    string DownloadUrl,
    string FileName,
    long FileSize,
    string ReleaseNotes);
