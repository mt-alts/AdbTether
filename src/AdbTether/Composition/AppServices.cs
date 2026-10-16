using AdbTether.Abstractions;
using AdbTether.Services;
using AdbTether.ViewModels;
using Microsoft.UI.Xaml;

namespace AdbTether.Composition;

/// <summary>
/// Hand-rolled composition root: builds every service once and wires the shell
/// view model. This is the only place that knows the concrete implementations,
/// which keeps the rest of the app depending on abstractions.
/// </summary>
public sealed class AppServices
{
    public AppServices(FrameworkElement root)
    {
        Core = new RemoteCoreService();
        Core.Init();

        // DNS resolution is always done by ws-proxy on the device; the mode
        // toggle is intentionally not exposed in the UI.
        Core.SetDnsMode(0);

        Adb = new AdbService();
        Appearance = new AppearanceService(root);
        Updates = new UpdateService();
        UpdatePreferences = new UpdatePreferences();
        Clipboard = new ClipboardService();
        Dialogs = new DialogService(UpdatePreferences);
        Tunnel = new TunnelConnector(Adb);
        Notifications = new NotificationService();
    }

    public ICoreService Core { get; }
    public IAdbService Adb { get; }
    public IAppearanceService Appearance { get; }
    public IUpdateService Updates { get; }
    public IUpdatePreferences UpdatePreferences { get; }
    public IClipboardService Clipboard { get; }
    public IDialogService Dialogs { get; }
    public TunnelConnector Tunnel { get; }
    public INotificationService Notifications { get; }

    /// <summary>Builds the shell view model and its child view models.</summary>
    public MainViewModel CreateMainViewModel()
    {
        var status = new StatusViewModel();
        var traffic = new TrafficViewModel(Core);
        var device = new DeviceViewModel(Adb, Core, Dialogs);
        var settings = new SettingsViewModel(Core, Appearance);
        var update = new UpdateViewModel(Updates, Dialogs, UpdatePreferences);
        var log = new LogViewModel(Core, Clipboard);

        return new MainViewModel(Core, Tunnel, Notifications, status, traffic, device, settings, update, log);
    }
}
