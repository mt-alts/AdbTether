using System.IO;
using AdbTether.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace AdbTether.Pages;

public sealed partial class HelpPage : Page
{
    private bool _loaded;

    public HelpPage()
    {
        InitializeComponent();
        Loaded += OnLoaded;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        if (_loaded)
            return;

        _loaded = true;

        var path = Path.Combine(AppContext.BaseDirectory, "Assets", "Help.md");

        try
        {
            MarkdownRenderer.Render(File.ReadAllText(path), DocumentHost);
        }
        catch (Exception ex)
        {
            DocumentHost.Children.Add(new TextBlock
            {
                Text = $"Help content could not be loaded: {ex.Message}",
                TextWrapping = TextWrapping.Wrap,
            });
        }
    }
}
