using System.Numerics;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using AdbTether.ViewModels;

namespace AdbTether.Pages;

public sealed partial class SettingsPage : Page
{
    public MainViewModel ViewModel { get; private set; } = null!;

    public SettingsPage()
    {
        InitializeComponent();

        // The result snackbar has to read as floating above the page, and a
        // theme shadow only shows once the element is lifted off the plane. The
        // XAML compiler in this SDK build trips over Translation as an
        // attribute, so the lift is applied here instead.
        UpdateSnack.Translation = new Vector3(0, 0, 32);
    }

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        ViewModel = (MainViewModel)e.Parameter;
        DataContext = ViewModel;
    }
}
