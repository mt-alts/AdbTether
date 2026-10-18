using System.Collections.ObjectModel;
using AdbTether.Abstractions;
using AdbTether.Models;

namespace AdbTether.ViewModels;

/// <summary>
/// The Android device list plus the "add device" flows (TCP/IP connect and
/// pairing). Owns the short activity message shown under the status bar.
/// </summary>
public sealed class DeviceViewModel : ViewModelBase
{
    private readonly IAdbService _adb;
    private readonly ICoreService _core;
    private readonly IDialogService _dialogs;

    private AdbDevice? _selected;
    private string _message = "";

    public DeviceViewModel(IAdbService adb, ICoreService core, IDialogService dialogs)
    {
        _adb = adb;
        _core = core;
        _dialogs = dialogs;

        RefreshCommand = new AsyncRelayCommand(RefreshAsync);
        AddConnectCommand = new AsyncRelayCommand(AddViaConnectAsync);
        AddPairCommand = new AsyncRelayCommand(AddViaPairAsync);
    }

    public ObservableCollection<AdbDevice> Items { get; } = new();

    public AsyncRelayCommand RefreshCommand { get; }
    public AsyncRelayCommand AddConnectCommand { get; }
    public AsyncRelayCommand AddPairCommand { get; }

    public AdbDevice? Selected
    {
        get => _selected;
        set
        {
            if (SetProperty(ref _selected, value) && value is not null)
                _core.SetDevice(value.Serial);
        }
    }

    /// <summary>Short progress/result line for adb activity.</summary>
    public string Message
    {
        get => _message;
        set => SetProperty(ref _message, value);
    }

    public async Task RefreshAsync()
    {
        try
        {
            var devices = await _adb.GetDevicesAsync();
            Items.Clear();
            foreach (AdbDevice d in devices)
                Items.Add(d);
            if (Items.Count > 0)
                Selected = Items[0];

            // Clear a stale failure line, but leave connect/pair results alone.
            if (Message.StartsWith("Device scan failed", StringComparison.Ordinal))
                Message = "";
        }
        catch (Exception ex)
        {
            // Swallowing this used to leave an empty list and no reason for it.
            Items.Clear();
            Message = $"Device scan failed: {ex.Message}";
        }
    }

    private async Task AddViaConnectAsync()
    {
        var request = await _dialogs.ShowAdbConnectAsync();
        if (request is null)
            return;

        try
        {
            string result = await _adb.ConnectAsync(request.IpAddress, request.Port);
            Message = result.Contains("connected", StringComparison.OrdinalIgnoreCase)
                ? "Connected."
                : $"Connect failed: {result}";
        }
        catch (Exception ex)
        {
            Message = $"Connect error: {ex.Message}";
        }

        await RefreshAsync();
    }

    private async Task AddViaPairAsync()
    {
        var request = await _dialogs.ShowAdbPairAsync();
        if (request is null)
            return;

        try
        {
            string result = await _adb.PairAsync(request.IpAddress, request.Port, request.PairingCode);
            Message = result.Contains("success", StringComparison.OrdinalIgnoreCase)
                ? "Pairing OK."
                : $"Pairing failed: {result}";
        }
        catch (Exception ex)
        {
            Message = $"Pairing error: {ex.Message}";
        }

        await RefreshAsync();
    }
}
