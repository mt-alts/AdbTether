using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using AdbTether.ViewModels;

namespace AdbTether.Pages;

public sealed partial class HomePage : Page
{
    public MainViewModel ViewModel { get; private set; } = null!;

    public HomePage() => InitializeComponent();

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);
        ViewModel = (MainViewModel)e.Parameter;
        DataContext = ViewModel;
    }

    // MenuFlyout content does not inherit DataContext in WinUI, so these two
    // forward to the view-model commands instead of binding directly.
    private void AdbConnectMenu_Click(object sender, RoutedEventArgs e) =>
        ViewModel.Device.AddConnectCommand.Execute(null);

    private void AdbPairMenu_Click(object sender, RoutedEventArgs e) =>
        ViewModel.Device.AddPairCommand.Execute(null);
}
