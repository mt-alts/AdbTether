namespace AdbTether.Abstractions;

/// <summary>
/// User choices about update prompts that outlive a single run. Kept apart from
/// the engine settings because they only concern the UI.
/// </summary>
public interface IUpdatePreferences
{
    /// <summary>
    /// When true the app does not check for updates on launch, so the update
    /// prompt never appears at startup. The check on the Settings page is not
    /// affected and stays the way back in.
    /// </summary>
    bool SkipStartupPrompt { get; set; }
}
