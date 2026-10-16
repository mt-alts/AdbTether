using System.Collections.ObjectModel;
using AdbTether.Abstractions;

namespace AdbTether.ViewModels;

/// <summary>
/// A single bypass host. Each row owns its remove command so the list can bind
/// it without reaching back up to the page's data context.
/// </summary>
public sealed class FilterHostViewModel
{
    public FilterHostViewModel(string host, Action<string> remove)
    {
        Host = host;
        RemoveCommand = new RelayCommand(() => remove(Host));
    }

    public string Host { get; }

    public RelayCommand RemoveCommand { get; }
}

/// <summary>Theme, tunnel options and the tunnel bypass-host filter.</summary>
public sealed class SettingsViewModel : ViewModelBase
{
    private const int DefaultPort = 1092;
    private const int MinPort = 1;
    private const int MaxPort = 65535;

    private readonly ICoreService _core;
    private readonly IAppearanceService _appearance;

    private bool _ipv6Block;
    private int _serverPort = DefaultPort;
    private string? _newFilterHost;

    public SettingsViewModel(ICoreService core, IAppearanceService appearance)
    {
        _core = core;
        _appearance = appearance;

        _ipv6Block = core.GetIpv6Block() == 1;
        _serverPort = core.GetPort();
        LoadFilterHosts();

        AddFilterHostCommand = new RelayCommand(AddFilterHost);
    }

    public IReadOnlyList<string> ThemeNames { get; } = new[] { "system", "light", "dark" };

    public string SelectedTheme
    {
        get => _appearance.CurrentTheme;
        set
        {
            if (value != _appearance.CurrentTheme)
            {
                _appearance.Apply(value);
                OnPropertyChanged();
            }
        }
    }

    /// <summary>
    /// True = drop all IPv6 while connected (kill-switch, default);
    /// false = leave IPv6 on the physical network (leaks AAAA/DNS-v6/WebRTC).
    /// </summary>
    public bool Ipv6Block
    {
        get => _ipv6Block;
        set
        {
            if (SetProperty(ref _ipv6Block, value))
                _core.SetIpv6Block(value ? 1 : 0);
        }
    }

    /// <summary>Adb forward + proxy port used by the engine.</summary>
    public int ServerPort
    {
        get => _serverPort;
        set
        {
            int clamped = Math.Clamp(value, MinPort, MaxPort);
            if (SetProperty(ref _serverPort, clamped))
                _core.SetPort((ushort)clamped);
        }
    }

    /// <summary>Hosts (domains/IPs) that bypass the tunnel.</summary>
    public ObservableCollection<FilterHostViewModel> FilterHosts { get; } = new();

    public string? NewFilterHost
    {
        get => _newFilterHost;
        set => SetProperty(ref _newFilterHost, value);
    }

    public RelayCommand AddFilterHostCommand { get; }

    private void AddFilterHost()
    {
        string host = (NewFilterHost ?? "").Trim();
        if (host.Length == 0 || FilterHosts.Any(h => h.Host == host))
            return;

        FilterHosts.Add(new FilterHostViewModel(host, RemoveFilterHost));
        NewFilterHost = "";
        SyncFilterHosts();
    }

    private void RemoveFilterHost(string host)
    {
        var match = FilterHosts.FirstOrDefault(h => h.Host == host);
        if (match is null)
            return;

        FilterHosts.Remove(match);
        SyncFilterHosts();
    }

    private void SyncFilterHosts() =>
        _core.SetFilterHosts(string.Join(",", FilterHosts.Select(h => h.Host)));

    private void LoadFilterHosts()
    {
        string raw = _core.GetFilterHosts();
        foreach (string host in raw.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            if (FilterHosts.All(h => h.Host != host))
                FilterHosts.Add(new FilterHostViewModel(host, RemoveFilterHost));
        }
    }
}
