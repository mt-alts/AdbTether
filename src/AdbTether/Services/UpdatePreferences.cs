using System.Text.Json;
using AdbTether.Abstractions;

namespace AdbTether.Services;

/// <summary>
/// Persists the update-prompt preferences under the user's local app data.
/// The installed app lives in Program Files, so the file cannot sit next to the
/// executable the way the appearance file does; a missing or unreadable file
/// simply means the defaults.
/// </summary>
public sealed class UpdatePreferences : IUpdatePreferences
{
    private const string FolderName = "AdbTether";
    private const string FileName = "update-preferences.json";

    /// <summary>On-disk shape; kept as a record so existing files stay valid.</summary>
    private sealed record UpdatePreferencesState(bool SkipStartupPrompt);

    private readonly string _path = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        FolderName,
        FileName);

    private bool _skipStartupPrompt;

    public UpdatePreferences() => Load();

    public bool SkipStartupPrompt
    {
        get => _skipStartupPrompt;
        set
        {
            if (_skipStartupPrompt == value)
                return;

            _skipStartupPrompt = value;
            Save();
        }
    }

    private void Load()
    {
        try
        {
            if (File.Exists(_path))
            {
                string json = File.ReadAllText(_path);
                var state = JsonSerializer.Deserialize<UpdatePreferencesState>(json);
                if (state is not null)
                    _skipStartupPrompt = state.SkipStartupPrompt;
            }
        }
        catch
        {
            // Corrupt or unreadable -> keep defaults.
            _skipStartupPrompt = false;
        }
    }

    private void Save()
    {
        try
        {
            Directory.CreateDirectory(Path.GetDirectoryName(_path)!);
            File.WriteAllText(_path, JsonSerializer.Serialize(new UpdatePreferencesState(_skipStartupPrompt)));
        }
        catch
        {
            // Best effort.
        }
    }
}
