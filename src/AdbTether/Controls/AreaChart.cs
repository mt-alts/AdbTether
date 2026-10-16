using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.UI;

namespace AdbTether.Controls;

/// <summary>
/// Lightweight overlapping area chart for WinUI. Renders two time series
/// (uplink/downlink) as filled polygons that overlap, with a subtle baseline.
/// Values are raw Mbps and are auto-scaled to the current window peak so the
/// chart stays dynamic at any bandwidth.
/// </summary>
public sealed class AreaChart : Grid
{
    private readonly Canvas _canvas;
    private readonly Polygon _uplinkArea;
    private readonly Polygon _downlinkArea;

    private static readonly Color UplinkColor = Color.FromArgb(255, 0x22, 0xD3, 0xEE);
    private static readonly Color DownlinkColor = Color.FromArgb(255, 0x22, 0xC5, 0x5E);

    /// Floor used so a series of all-zeros still renders at the baseline.
    private const double PeakFloorMbps = 0.05;

    public AreaChart()
    {
        _canvas = new Canvas();

        _downlinkArea = new Polygon
        {
            Fill = new SolidColorBrush(Color.FromArgb(110, DownlinkColor.R, DownlinkColor.G, DownlinkColor.B)),
            Stroke = new SolidColorBrush(DownlinkColor),
            StrokeThickness = 1.5,
        };
        _uplinkArea = new Polygon
        {
            Fill = new SolidColorBrush(Color.FromArgb(140, UplinkColor.R, UplinkColor.G, UplinkColor.B)),
            Stroke = new SolidColorBrush(UplinkColor),
            StrokeThickness = 1.5,
        };

        _canvas.Children.Add(_downlinkArea);
        _canvas.Children.Add(_uplinkArea);
        Children.Add(_canvas);
        SizeChanged += (_, _) => Redraw();
    }

    public static readonly DependencyProperty UplinkSeriesProperty = DependencyProperty.Register(
        nameof(UplinkSeries), typeof(IReadOnlyList<double>), typeof(AreaChart),
        new PropertyMetadata(null, OnSeriesChanged));

    public static readonly DependencyProperty DownlinkSeriesProperty = DependencyProperty.Register(
        nameof(DownlinkSeries), typeof(IReadOnlyList<double>), typeof(AreaChart),
        new PropertyMetadata(null, OnSeriesChanged));

    public IReadOnlyList<double>? UplinkSeries
    {
        get => (IReadOnlyList<double>?)GetValue(UplinkSeriesProperty);
        set => SetValue(UplinkSeriesProperty, value);
    }

    public IReadOnlyList<double>? DownlinkSeries
    {
        get => (IReadOnlyList<double>?)GetValue(DownlinkSeriesProperty);
        set => SetValue(DownlinkSeriesProperty, value);
    }

    private static void OnSeriesChanged(DependencyObject d, DependencyPropertyChangedEventArgs e)
    {
        if (d is AreaChart chart)
            chart.Redraw();
    }

    private void Redraw()
    {
        double w = ActualWidth;
        double h = ActualHeight;
        if (w <= 0 || h <= 0)
            return;

        _canvas.Width = w;
        _canvas.Height = h;

        // Auto-scale to the peak across both series so the chart stays
        // dynamic at any real bandwidth.
        double peak = Math.Max(PeakOf(UplinkSeries), PeakOf(DownlinkSeries));
        if (peak < PeakFloorMbps)
            peak = PeakFloorMbps;

        _uplinkArea.Points = BuildPoints(UplinkSeries, w, h, peak);
        _downlinkArea.Points = BuildPoints(DownlinkSeries, w, h, peak);
    }

    private static double PeakOf(IReadOnlyList<double>? series)
    {
        double peak = 0;
        if (series == null)
            return peak;
        foreach (double v in series)
        {
            if (v > peak)
                peak = v;
        }
        return peak;
    }

    private static PointCollection BuildPoints(IReadOnlyList<double>? series, double w, double h, double peak)
    {
        var points = new PointCollection();
        if (series == null || series.Count < 2)
            return points;

        int count = series.Count;
        points.Add(new Windows.Foundation.Point(0, h));
        for (int i = 0; i < count; i++)
        {
            double x = w * i / (count - 1.0);
            double v = Math.Clamp(series[i] / peak, 0, 1);
            double y = h * (1.0 - v);
            points.Add(new Windows.Foundation.Point(x, y));
        }
        points.Add(new Windows.Foundation.Point(w, h));
        return points;
    }
}
