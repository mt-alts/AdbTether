using System.Diagnostics;
using System.IO;
using System.Net.Http;
using System.Reflection;
using System.Text.Json;
using AdbTether.Abstractions;
using AdbTether.Models;

namespace AdbTether.Services;

/// <summary>
/// Checks for updates via the GitHub releases API and downloads/installs new versions.
/// </summary>
public sealed class UpdateService : IUpdateService
{
    private const string GitHubApiUrl = "https://api.github.com/repos/mt-alts/AdbTether/releases/latest";
    private const string UserAgent = "AdbTether";
    private const string MsiSuffix = ".msi";
    private const string DefaultMsiName = "AdbTether.msi";
    private const string MsiexecExe = "msiexec.exe";
    private const string PassiveArg = "/passive";
    private const string TempFolder = "AdbTether";
    private const int DownloadBufferSize = 8192;

    /// <summary>Checks GitHub for a newer release. Returns null if up-to-date.</summary>
    public async Task<UpdateInfo?> CheckForUpdateAsync(CancellationToken ct = default)
    {
        using var client = new HttpClient();
        client.DefaultRequestHeaders.UserAgent.ParseAdd(UserAgent);

        string json = await client.GetStringAsync(GitHubApiUrl, ct);
        using var doc = JsonDocument.Parse(json);
        var root = doc.RootElement;

        string tag = root.GetProperty("tag_name").GetString() ?? "";
        string htmlUrl = root.GetProperty("html_url").GetString() ?? "";
        string releaseNotes = root.TryGetProperty("body", out var body) ? body.GetString() ?? "" : "";

        var current = Assembly.GetExecutingAssembly().GetName().Version;
        string latestStr = tag.TrimStart('v');

        if (!Version.TryParse(latestStr, out var latest) || current is null || !(latest > current))
            return null; // Up to date (or unparsable tag)

        // Find the installer (.msi) in the release assets.
        string downloadUrl = "";
        string fileName = DefaultMsiName;
        long fileSize = 0;

        if (root.TryGetProperty("assets", out var assets))
        {
            for (int i = 0; i < assets.GetArrayLength(); i++)
            {
                var asset = assets[i];
                string name = asset.GetProperty("name").GetString() ?? "";
                if (name.EndsWith(MsiSuffix, StringComparison.OrdinalIgnoreCase))
                {
                    downloadUrl = asset.GetProperty("browser_download_url").GetString() ?? "";
                    fileName = name;
                    fileSize = asset.GetProperty("size").GetInt64();
                    break;
                }
            }
        }

        return new UpdateInfo(tag, htmlUrl, downloadUrl, fileName, fileSize, releaseNotes);
    }

    /// <summary>Downloads the update MSI to temp and launches it via msiexec.</summary>
    public async Task DownloadAndInstallAsync(
        UpdateInfo info,
        IProgress<double>? progress = null,
        CancellationToken ct = default)
    {
        if (string.IsNullOrEmpty(info.DownloadUrl))
            throw new InvalidOperationException("This release has no installer (.msi) asset.");

        using var client = new HttpClient();
        client.DefaultRequestHeaders.UserAgent.ParseAdd(UserAgent);

        string destDir = Path.Combine(Path.GetTempPath(), TempFolder);
        Directory.CreateDirectory(destDir);
        string destPath = Path.Combine(destDir, info.FileName);

        using (var response = await client.GetAsync(info.DownloadUrl, HttpCompletionOption.ResponseHeadersRead, ct))
        {
            response.EnsureSuccessStatusCode();
            using var contentStream = await response.Content.ReadAsStreamAsync(ct);
            using var fileStream = File.Create(destPath);

            var buffer = new byte[DownloadBufferSize];
            long totalRead = 0;
            int bytesRead;

            while ((bytesRead = await contentStream.ReadAsync(buffer, ct)) > 0)
            {
                await fileStream.WriteAsync(buffer.AsMemory(0, bytesRead), ct);
                totalRead += bytesRead;

                if (info.FileSize > 0)
                    progress?.Report(5.0 + (double)totalRead / info.FileSize * 90.0);
            }
        }

        progress?.Report(100);

        // Run the MSI unattended: /passive shows only a progress bar. The package is
        // per-machine (it installs the service), so runas raises the UAC prompt it needs.
        Process.Start(new ProcessStartInfo(MsiexecExe, $"/i \"{destPath}\" {PassiveArg}")
        {
            UseShellExecute = true,
            Verb = "runas",
            WorkingDirectory = Path.GetTempPath(),
        });
    }
}
