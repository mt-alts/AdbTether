using AdbTether.Abstractions;

namespace AdbTether.ViewModels;

/// <summary>
/// Samples the engine's byte counters once per second and exposes the current
/// rates plus the rolling series the area chart draws.
/// </summary>
public sealed class TrafficViewModel : ViewModelBase
{
    private const int HistoryLength = 90;
    private const double MinSampleSeconds = 1.0;
    private const double MinElapsedSeconds = 0.05;

    private readonly ICoreService _core;
    private readonly double[] _uplinkHistory = new double[HistoryLength];
    private readonly double[] _downlinkHistory = new double[HistoryLength];

    private int _historyPos;
    private double[] _uplinkSeries = Array.Empty<double>();
    private double[] _downlinkSeries = Array.Empty<double>();
    private DateTime _lastSample = DateTime.MinValue;
    private long _lastUpBytes;
    private long _lastDownBytes;
    private string _uplinkValue = "0 KB/s";
    private string _downlinkValue = "0 KB/s";

    public TrafficViewModel(ICoreService core) => _core = core;

    public string UplinkValue
    {
        get => _uplinkValue;
        private set => SetProperty(ref _uplinkValue, value);
    }

    public string DownlinkValue
    {
        get => _downlinkValue;
        private set => SetProperty(ref _downlinkValue, value);
    }

    /// <summary>Time series of uplink levels (Mbps) for the area chart, oldest first.</summary>
    public IReadOnlyList<double> UplinkSeries => _uplinkSeries;

    /// <summary>Time series of downlink levels (Mbps) for the area chart, oldest first.</summary>
    public IReadOnlyList<double> DownlinkSeries => _downlinkSeries;

    /// <summary>Called on every UI tick; samples at most once per second.</summary>
    public void Sample()
    {
        DateTime now = DateTime.UtcNow;
        if ((now - _lastSample).TotalSeconds < MinSampleSeconds)
            return;

        double elapsed = (now - _lastSample).TotalSeconds;
        if (elapsed < MinElapsedSeconds)
            return;

        var (upBytes, downBytes) = _core.GetStats();
        double upMbps = (upBytes - _lastUpBytes) * 8.0 / 1_000_000.0 / elapsed;
        double downMbps = (downBytes - _lastDownBytes) * 8.0 / 1_000_000.0 / elapsed;

        _lastUpBytes = upBytes;
        _lastDownBytes = downBytes;
        _lastSample = now;

        UplinkValue = FormatMbps(upMbps);
        DownlinkValue = FormatMbps(downMbps);
        PushHistory(upMbps, downMbps);
    }

    /// <summary>Starts a fresh measurement window (called on every tunnel toggle).</summary>
    public void Reset()
    {
        _lastUpBytes = 0;
        _lastDownBytes = 0;
        _lastSample = DateTime.UtcNow;
    }

    private void PushHistory(double upMbps, double downMbps)
    {
        // Keep raw Mbps values; the chart auto-scales to the current peak so
        // it stays dynamic regardless of the actual bandwidth in use.
        _uplinkHistory[_historyPos] = upMbps;
        _downlinkHistory[_historyPos] = downMbps;
        _historyPos = (_historyPos + 1) % HistoryLength;

        var up = new double[HistoryLength];
        var down = new double[HistoryLength];
        for (int i = 0; i < HistoryLength; i++)
        {
            up[i] = _uplinkHistory[(_historyPos + i) % HistoryLength];
            down[i] = _downlinkHistory[(_historyPos + i) % HistoryLength];
        }

        _uplinkSeries = up;
        _downlinkSeries = down;
        OnPropertyChanged(nameof(UplinkSeries));
        OnPropertyChanged(nameof(DownlinkSeries));
    }

    private static string FormatMbps(double mbps) =>
        mbps >= 100 ? $"{(mbps / 1000):0.00} Gbps"
        : mbps >= 1 ? $"{mbps:0.00} Mbps"
        : $"{(mbps * 1000):0} Kbps";
}
