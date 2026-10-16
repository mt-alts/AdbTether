namespace AdbTether.Abstractions;

/// <summary>
/// Raises user-facing notifications on behalf of view models, so no view model
/// touches the platform notification APIs directly.
/// </summary>
public interface INotificationService
{
    /// <summary>True when the platform accepted registration and toasts can be shown.</summary>
    bool IsAvailable { get; }

    /// <summary>Shows a notification. Best effort: never throws.</summary>
    void Notify(string title, string message);
}
