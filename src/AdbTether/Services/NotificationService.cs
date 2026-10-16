using AdbTether.Abstractions;
using Microsoft.Windows.AppNotifications;
using Microsoft.Windows.AppNotifications.Builder;

namespace AdbTether.Services;

/// <summary>
/// Windows toast notifications via the Windows App SDK. This works for the
/// unpackaged desktop app because <c>AppNotificationManager</c> registers the
/// process on startup (no MSIX identity required).
/// </summary>
/// <remarks>
/// Every call is best-effort: a notification failure must never break the UI.
/// When registration fails, <see cref="IsAvailable"/> is false and the in-app
/// status line still reflects the condition.
/// </remarks>
public sealed class NotificationService : INotificationService
{
    public NotificationService()
    {
        try
        {
            AppNotificationManager.Default.Register();
            IsAvailable = true;
        }
        catch
        {
            IsAvailable = false;
        }
    }

    public bool IsAvailable { get; }

    public void Notify(string title, string message)
    {
        if (!IsAvailable)
            return;

        try
        {
            var notification = new AppNotificationBuilder()
                .AddText(title)
                .AddText(message)
                .BuildNotification();
            AppNotificationManager.Default.Show(notification);
        }
        catch
        {
            // Best effort: the in-app status line still reflects the condition.
        }
    }
}
