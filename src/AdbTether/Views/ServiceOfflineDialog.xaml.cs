using Microsoft.UI.Xaml;

namespace AdbTether.Views;

/// <summary>
/// Warning shown when the background service is unreachable, together with the
/// commands needed to start or install it. The primary button asks the caller
/// to re-check availability.
/// </summary>
public sealed partial class ServiceOfflineDialog : ThemedContentDialog
{
    public ServiceOfflineDialog(string detail)
    {
        InitializeComponent();

        bool hasDetail = !string.IsNullOrWhiteSpace(detail);
        DetailText.Text = hasDetail ? $"Details: {detail}" : "";
        DetailText.Visibility = hasDetail ? Visibility.Visible : Visibility.Collapsed;
    }
}
