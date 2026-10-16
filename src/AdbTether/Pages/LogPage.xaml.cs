using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using AdbTether.ViewModels;

namespace AdbTether.Pages;

public sealed partial class LogPage : Page
{
    public LogViewModel ViewModel { get; private set; } = null!;

    public LogPage() => InitializeComponent();

    protected override void OnNavigatedTo(NavigationEventArgs e)
    {
        base.OnNavigatedTo(e);

        // The log view model is owned by the shell, so its timer survives
        // navigation; we only hook the view up to it while this page is live.
        ViewModel = (LogViewModel)e.Parameter;
        DataContext = ViewModel;
        ViewModel.AutoScrollRequested += OnAutoScrollRequested;
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        ViewModel.AutoScrollRequested -= OnAutoScrollRequested;
    }

    private void OnAutoScrollRequested() =>
        LogScroll.ChangeView(null, LogScroll.ScrollableHeight, null);

    private void LogScroll_ViewChanged(object? sender, ScrollViewerViewChangedEventArgs e)
    {
        bool atBottom = LogScroll.VerticalOffset >= LogScroll.ScrollableHeight - 4;
        ViewModel.MarkAtBottom(atBottom);
    }
}
