using System.Text.Json;
using AdbTether.Abstractions;
using Microsoft.UI.Xaml;

namespace AdbTether.Services;

/// <summary>
/// Persists and applies the app theme. Persisted in a JSON file next to the
/// executable (works for the unpackaged WinUI 3 app). Applied via the root
/// grid's RequestedTheme.
/// </summary>
public sealed class AppearanceService : IAppearanceService
{
    private const string FileName = "AdbTether.appearance.json";
    private const string DefaultTheme = "system";

    /// <summary>On-disk shape; kept as a record so existing settings files stay valid.</summary>
    private sealed record AppearanceState(string Theme);

    private readonly string _path = Path.Combine(AppContext.BaseDirectory, FileName);
    private readonly FrameworkElement? _root;

    private string _theme = DefaultTheme;

    public AppearanceService(FrameworkElement? root)
    {
        _root = root;
        Load();
    }

    public string CurrentTheme => _theme;

    /// <summary>Applies the theme immediately and persists it.</summary>
    public void Apply(string theme)
    {
        _theme = theme;
        ApplyTheme(theme);
        Save();
    }

    private void Load()
    {
        try
        {
            if (File.Exists(_path))
            {
                string json = File.ReadAllText(_path);
                var state = JsonSerializer.Deserialize<AppearanceState>(json);
                if (state is not null)
                    _theme = state.Theme;
            }
        }
        catch
        {
            // Corrupt or unreadable -> keep defaults.
            _theme = DefaultTheme;
        }
    }

    private void Save()
    {
        try
        {
            File.WriteAllText(_path, JsonSerializer.Serialize(new AppearanceState(_theme)));
        }
        catch
        {
            // Best effort.
        }
    }

    private void ApplyTheme(string theme)
    {
        if (_root is null)
            return;

        _root.RequestedTheme = theme switch
        {
            "dark" => ElementTheme.Dark,
            "light" => ElementTheme.Light,
            _ => ElementTheme.Default,
        };
    }
}
