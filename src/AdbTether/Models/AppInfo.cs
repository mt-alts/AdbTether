using System.Reflection;

namespace AdbTether.Models;

/// <summary>
/// Single source of truth for static application metadata. Kept out of the
/// services so views, view models and services all agree on one value.
/// </summary>
public static class AppInfo
{
    public const string Name = "AdbTether";

    /// <summary>Public repository URL (shown on the About page).</summary>
    public const string RepositoryUrl = "https://www.github.com/mt-alts/AdbTether";

    public static string CurrentVersion =>
        Assembly.GetExecutingAssembly().GetName().Version?.ToString(3) ?? "0.1.0";
}
