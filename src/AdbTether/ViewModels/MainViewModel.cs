using AdbTether.Abstractions;
using AdbTether.Models;
using AdbTether.Services;
using Microsoft.UI.Dispatching;

namespace AdbTether.ViewModels;

/// <summary>
/// The shell view model. It composes the focused view models, polls the engine
/// on a timer and owns the tunnel connect/disconnect command. Business logic
/// lives in the child view models and services; this type only coordinates.
/// </summary>
public sealed class MainViewModel : ViewModelBase, IDisposable
{
    private const int PollIntervalMs = 300;

    private readonly ICoreService _core;
    private readonly TunnelConnector _connector;
    private readonly INotificationService _notifications;
    private readonly DispatcherQueueTimer _timer;

    private bool _isConnected;
    private bool _disposed;
    private bool _wasTunnelActive;
    private bool _lossNotified;

    public MainViewModel(
        ICoreService core,
        TunnelConnector connector,
        INotificationService notifications,
        StatusViewModel status,
        TrafficViewModel traffic,
        DeviceViewModel device,
        SettingsViewModel settings,
        UpdateViewModel update,
        LogViewModel log)
    {
        _core = core;
        _connector = connector;
        _notifications = notifications;

        Status = status;
        Traffic = traffic;
        Device = device;
        Settings = settings;
        Update = update;
        Log = log;

        ConnectCommand = new AsyncRelayCommand(ConnectAsync);

        // AdbMessage is the activity line shown by the shell; it is owned by the
        // device view model (which also produces most of its text).
        Device.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(DeviceViewModel.Message))
                OnPropertyChanged(nameof(AdbMessage));
        };

        if (!_core.IsAvailable)
            Status.SetMessage(_core.LastError, TunnelState.Error);

        _timer = DispatcherQueue.GetForCurrentThread().CreateTimer();
        _timer.Interval = TimeSpan.FromMilliseconds(PollIntervalMs);
        _timer.IsRepeating = true;
        _timer.Tick += (_, _) => OnTick();
        _timer.Start();

        _ = Device.RefreshAsync();
    }

    public StatusViewModel Status { get; }
    public TrafficViewModel Traffic { get; }
    public DeviceViewModel Device { get; }
    public SettingsViewModel Settings { get; }
    public UpdateViewModel Update { get; }
    public LogViewModel Log { get; }

    public AsyncRelayCommand ConnectCommand { get; }

    /// <summary>Short activity line shown under the status bar.</summary>
    public string AdbMessage => Device.Message;

    public bool IsConnected
    {
        get => _isConnected;
        private set
        {
            if (SetProperty(ref _isConnected, value))
                OnPropertyChanged(nameof(ConnectLabel));
        }
    }

    public string ConnectLabel => IsConnected ? "DISCONNECT" : "CONNECT";

    private void OnTick()
    {
        try
        {
            string status = _core.GetStatus();
            bool tunnelActive = _core.IsTunnelActive();

            // The engine keeps the tunnel up (fail-closed) when the ws-proxy
            // connection drops, so remember the cached proxy is stale and the
            // next connect must re-establish it.
            bool lost = StatusClassifier.IsConnectionLost(status);
            if (lost)
                _connector.MarkProxyUnreachable();

            if (lost)
            {
                // Notify once per drop, and only after the tunnel was actually up
                // in this session, so a stale status at startup does not fire.
                if (_wasTunnelActive && !_lossNotified)
                {
                    _lossNotified = true;
                    string device = string.IsNullOrEmpty(Device.Selected) ? "the device" : Device.Selected;
                    _notifications.Notify(
                        "AdbTether - connection lost",
                        $"ws-proxy on {device} stopped responding. The PC stays on the tunnel without internet until you disconnect.");
                }
                _wasTunnelActive = false;
            }
            else if (tunnelActive)
            {
                _wasTunnelActive = true;
                _lossNotified = false;
            }

            IsConnected = _core.IsConnected() || tunnelActive;
            Status.Update(status, tunnelActive);
            Traffic.Sample();
        }
        catch (Exception ex)
        {
            // Service unavailable: surface once, then keep polling quietly.
            string message = $"Service offline: {ex.Message}";
            if (Status.Text != message)
                Status.SetMessage(message, TunnelState.Error);
        }
    }

    private async Task ConnectAsync()
    {
        string device = Device.Selected ?? "";
        int port = Settings.ServerPort;

        // Disconnect: suspend the tunnel but KEEP the wintun adapter and the
        // on-device proxy running so the next connect resumes instantly.
        if (IsConnected)
        {
            await Task.Run(() => _core.ToggleTunnel());
            Device.Message = "";
            return;
        }

        // Connect. On the very first connect we set up the adb forward and start
        // the on-device proxy. On a re-connect the proxy is still alive (we never
        // kill it on disconnect), so we skip straight to resume.
        if (!_connector.IsProxyReady)
        {
            try
            {
                var progress = new Progress<string>(message => Device.Message = message);
                bool ready = await _connector.EnsureProxyAsync(device, port, progress);
                if (!ready)
                {
                    string message = "Proxy not reachable on 127.0.0.1:" + port;
                    Status.SetMessage(message, TunnelState.Error);
                    Device.Message = message;
                    return;
                }
                Device.Message = "";
            }
            catch (Exception ex)
            {
                string message = $"ADB setup failed: {ex.Message}";
                Status.SetMessage(message, TunnelState.Error);
                Device.Message = message;
                return;
            }
        }

        _core.SetDevice(device);
        _core.ToggleTunnel();
        // Start a fresh measurement window on every toggle.
        _core.ResetStats();
        Traffic.Reset();
    }

    public void Dispose()
    {
        if (_disposed)
            return;
        _disposed = true;

        _timer.Stop();
        Log.Dispose();

        // Kill ws-proxy on the device before destroying the tunnel.
        if (!string.IsNullOrEmpty(Device.Selected))
        {
            try
            {
                _connector.KillProxyAsync(Device.Selected).Wait(TimeSpan.FromSeconds(3));
            }
            catch { /* best effort */ }
        }
    }
}
