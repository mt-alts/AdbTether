using System.Collections.ObjectModel;
using AdbTether.Abstractions;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace AdbTether.ViewModels;

/// <summary>One coloured line in the log view.</summary>
public sealed class LogEntry
{
    public required string Text { get; init; }
    public required Brush Brush { get; init; }
}

/// <summary>
/// Polls the engine log and exposes it as a bounded, colour-coded list.
/// </summary>
public sealed class LogViewModel : ViewModelBase, IDisposable
{
    private const int MaxEntries = 2000;
    private static readonly TimeSpan PollInterval = TimeSpan.FromMilliseconds(250);

    private static readonly Brush NormalBrush = new SolidColorBrush(Color.FromArgb(255, 0xC9, 0xD2, 0xDC));
    private static readonly Brush WarnBrush = new SolidColorBrush(Color.FromArgb(255, 0xFB, 0xBF, 0x24));
    private static readonly Brush ErrorBrush = new SolidColorBrush(Color.FromArgb(255, 0xF8, 0x71, 0x71));
    private static readonly Brush InfoBrush = new SolidColorBrush(Color.FromArgb(255, 0x7D, 0xD3, 0xFC));

    private readonly ICoreService _core;
    private readonly IClipboardService _clipboard;
    private readonly DispatcherQueueTimer _timer;
    private bool _isAtBottom = true;

    public LogViewModel(ICoreService core, IClipboardService clipboard)
    {
        _core = core;
        _clipboard = clipboard;

        ClearCommand = new RelayCommand(Clear);
        CopyCommand = new RelayCommand(Copy);

        _timer = DispatcherQueue.GetForCurrentThread().CreateTimer();
        _timer.Interval = PollInterval;
        _timer.IsRepeating = true;
        _timer.Tick += (_, _) => Poll();
        _timer.Start();
        Poll();
    }

    public ObservableCollection<LogEntry> Entries { get; } = new();

    public RelayCommand ClearCommand { get; }
    public RelayCommand CopyCommand { get; }

    /// <summary>Raised when the view should scroll to the newest entry.</summary>
    public event Action? AutoScrollRequested;

    public void MarkAtBottom(bool atBottom) => _isAtBottom = atBottom;

    private void Poll()
    {
        foreach (string line in _core.GetLog())
            Entries.Add(new LogEntry { Text = line, Brush = Classify(line) });

        while (Entries.Count > MaxEntries)
            Entries.RemoveAt(0);

        if (_isAtBottom && Entries.Count > 0)
            AutoScrollRequested?.Invoke();
    }

    private static Brush Classify(string line)
    {
        if (line.Contains(" ERROR ") || line.Contains("Error") || line.Contains("error")
            || line.Contains("failed") || line.Contains("FAILED") || line.Contains("Offline"))
            return ErrorBrush;
        if (line.Contains(" WARN ") || line.Contains("WARNING") || line.Contains("waiting")
            || line.Contains("Waiting"))
            return WarnBrush;
        if (line.Contains(" INFO ") || line.StartsWith('[') || line.Contains("->") || line.Contains("<-"))
            return InfoBrush;
        return NormalBrush;
    }

    private void Clear()
    {
        _core.ClearLog();
        Entries.Clear();
    }

    private void Copy() =>
        _clipboard.SetText(string.Join(Environment.NewLine, Entries.Select(e => e.Text)));

    public void Dispose() => _timer.Stop();
}
