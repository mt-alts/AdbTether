using AdbTether.Models;
using AdbTether.Presentation;
using Microsoft.UI.Xaml.Media;

namespace AdbTether.ViewModels;

/// <summary>
/// Presentation of the tunnel status: the status line, the traffic indicator
/// and the colours that go with them. Owns no business logic beyond mapping.
/// </summary>
public sealed class StatusViewModel : ViewModelBase
{
    private string _text = "Ready";
    private Brush _dotBrush = StatusPresenter.IdleBrush;
    private string _tunnelText = "Tunnel Off";
    private Brush _tunnelDotBrush = StatusPresenter.IdleBrush;

    public string Text
    {
        get => _text;
        private set => SetProperty(ref _text, value);
    }

    public Brush DotBrush
    {
        get => _dotBrush;
        private set => SetProperty(ref _dotBrush, value);
    }

    public string TunnelText
    {
        get => _tunnelText;
        private set => SetProperty(ref _tunnelText, value);
    }

    public Brush TunnelDotBrush
    {
        get => _tunnelDotBrush;
        private set => SetProperty(ref _tunnelDotBrush, value);
    }

    /// <summary>Updates every field from one engine sample.</summary>
    public void Update(string rawStatus, bool tunnelActive)
    {
        Text = rawStatus;
        DotBrush = StatusPresenter.Present(StatusClassifier.Classify(rawStatus, tunnelActive)).DotBrush;
        TunnelText = tunnelActive ? "Tunnel Active" : "Tunnel Off";
        TunnelDotBrush = tunnelActive ? StatusPresenter.ActiveBrush : StatusPresenter.IdleBrush;
    }

    /// <summary>Shows a local, non-engine message (e.g. service offline).</summary>
    public void SetMessage(string message, TunnelState state)
    {
        Text = message;
        DotBrush = StatusPresenter.Present(state).DotBrush;
    }
}
