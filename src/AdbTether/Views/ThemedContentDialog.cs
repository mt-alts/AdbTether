using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace AdbTether.Views;

/// <summary>
/// Base for dialogs that inherit the shell's requested theme, so the
/// theme-sync boilerplate lives in exactly one place.
/// </summary>
public abstract class ThemedContentDialog : ContentDialog
{
    protected ThemedContentDialog()
    {
        Opened += (_, _) =>
        {
            if (XamlRoot?.Content is FrameworkElement root)
                RequestedTheme = root.RequestedTheme;
        };
    }
}
