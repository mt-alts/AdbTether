using AdbTether.Models;
using Microsoft.UI.Xaml.Media;
using Windows.UI;

namespace AdbTether.Presentation;

/// <summary>Visual attributes for a <see cref="TunnelState"/>.</summary>
public sealed record StatusVisual(string PillText, Brush PillBrush, Brush DotBrush);

/// <summary>
/// Owns the colour palette for tunnel state so view models never construct
/// brushes themselves and the theme lives in one place.
/// </summary>
public static class StatusPresenter
{
    public static Brush IdleBrush { get; } = new SolidColorBrush(Color.FromArgb(255, 0x8A, 0x8F, 0x98));
    public static Brush ActiveBrush { get; } = new SolidColorBrush(Color.FromArgb(255, 0x34, 0xD3, 0x99));
    public static Brush WorkingBrush { get; } = new SolidColorBrush(Color.FromArgb(255, 0xFB, 0xBF, 0x24));
    public static Brush ErrorBrush { get; } = new SolidColorBrush(Color.FromArgb(255, 0xF8, 0x71, 0x71));

    public static StatusVisual Present(TunnelState state) => state switch
    {
        TunnelState.Working => new StatusVisual("WORKING", WorkingBrush, WorkingBrush),
        TunnelState.Error => new StatusVisual("ERROR", ErrorBrush, ErrorBrush),
        TunnelState.Active => new StatusVisual("ACTIVE", ActiveBrush, ActiveBrush),
        _ => new StatusVisual("STANDBY", IdleBrush, IdleBrush),
    };
}
