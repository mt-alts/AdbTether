using System.Diagnostics;
using System.Threading;
using Microsoft.UI.Xaml;

namespace AdbTether;

public partial class App : Application
{
    private static Mutex? _singleInstanceMutex;
    private Window? _window;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, e) =>
        {
            try
            {
                File.AppendAllText(
                    Path.Combine(AppContext.BaseDirectory, "crash.log"),
                    $"[{DateTime.Now:HH:mm:ss}] {e.Exception}\n");
            }
            catch { }
            e.Handled = true;
        };
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _singleInstanceMutex = new Mutex(
            initiallyOwned: true,
            name: @"Local\AdbTether.SingleInstance",
            createdNew: out bool createdNew);

        if (!createdNew)
        {
            _singleInstanceMutex.Dispose();
            Environment.Exit(0);
            return;
        }

        _window = new MainWindow();
        _window.Activate();
    }
}
