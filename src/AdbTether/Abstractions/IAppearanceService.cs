namespace AdbTether.Abstractions;

/// <summary>Persists and applies the app theme.</summary>
public interface IAppearanceService
{
    string CurrentTheme { get; }

    void Apply(string theme);
}
