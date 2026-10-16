using AdbTether.Composition;
using AdbTether.Pages;
using AdbTether.ViewModels;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.UI;

namespace AdbTether;

/// <summary>
/// The shell window. Builds the composition root, hosts navigation and wires the
/// custom title bar; it contains no business logic.
/// </summary>
public sealed partial class MainWindow : Window
{
    private readonly AppServices _services;

    public MainViewModel ViewModel { get; }

    public MainWindow()
    {
        InitializeComponent();

        _services = new AppServices(RootGrid);
        _services.Appearance.Apply(_services.Appearance.CurrentTheme);

        ViewModel = _services.CreateMainViewModel();

        // Custom title bar: content extends, WinUI caption buttons handle min/close.
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBar);

        var titleBar = AppWindow.TitleBar;
        titleBar.ButtonBackgroundColor = Colors.Transparent;
        titleBar.ButtonInactiveBackgroundColor = Colors.Transparent;
        titleBar.ButtonForegroundColor = Color.FromArgb(255, 0xC8, 0xBC, 0xF0);
        titleBar.ButtonInactiveForegroundColor = Color.FromArgb(200, 0x8F, 0x81, 0xBD);

        // Hide maximize; keep only minimize + close.
        if (AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.IsMaximizable = false;
            presenter.IsResizable = false;
        }

        // Portrait window: narrow width, tall height.
        AppWindow.Resize(new Windows.Graphics.SizeInt32(650, 670));

        // The XamlRoot only exists once the shell is loaded, and the update
        // prompt needs it, so wire the dialogs and kick off the startup update
        // check from there (otherwise a fast check would be dropped).
        RootGrid.Loaded += (_, _) =>
        {
            _services.Dialogs.Initialize(RootGrid.XamlRoot);

            // Defer past the Loaded callback so the dialog has a live XamlRoot,
            // and check the service first so the two dialogs never overlap.
            RootGrid.DispatcherQueue.TryEnqueue(async () =>
            {
                await EnsureServiceAvailableAsync();
                _ = ViewModel.Update.CheckAsync(userInitiated: false);
            });
        };

        // Ensure tunnel + proxy are cleaned up on window close.
        Closed += (_, _) => ViewModel.Dispose();
    }

    private void NavView_Loaded(object sender, RoutedEventArgs e)
    {
        AppWindow.TitleBar.BackgroundColor = Colors.Coral;

        // Hide the auto-generated built-in Settings item (footer).
        NavView.IsSettingsVisible = false;

        NavView.SelectedItem = NavView.MenuItems[0];
        Navigate(AppPage.Home);
    }

    /// <summary>
    /// The UI is a thin RPC client, so with the service down nothing works. Warn
    /// with the fix (and a retry) instead of letting the shell look frozen.
    /// </summary>
    private async Task EnsureServiceAvailableAsync()
    {
        while (!_services.Core.IsAvailable)
        {
            bool retry = await _services.Dialogs.ShowServiceOfflineAsync(_services.Core.LastError);
            if (!retry)
                return;

            _services.Core.Init();
        }
    }

    private void NavView_SelectionChanged(
        NavigationView sender,
        NavigationViewSelectionChangedEventArgs args)
    {
        if (args.SelectedItem is NavigationViewItem { Tag: string tag } &&
            Enum.TryParse<AppPage>(tag, ignoreCase: true, out var page))
        {
            Navigate(page);
        }
    }

    private void Navigate(AppPage page)
    {
        switch (page)
        {
            case AppPage.Home:
                ContentFrame.Navigate(typeof(HomePage), ViewModel);
                break;
            case AppPage.Log:
                ContentFrame.Navigate(typeof(LogPage), ViewModel.Log);
                break;
            case AppPage.About:
                ContentFrame.Navigate(typeof(AboutPage));
                break;
            case AppPage.Settings:
                ContentFrame.Navigate(typeof(SettingsPage), ViewModel);
                break;
        }
    }

    /// <summary>Pages reachable from the navigation view; names match the XAML tags.</summary>
    private enum AppPage
    {
        Home,
        Log,
        About,
        Settings,
    }
}
