using System.ServiceProcess;

namespace AdbTetherService;

internal static class Program
{
    public static int Main(string[] args)
    {
        bool asService = args.Length > 0 && (args[0] == "--service" || args[0] == "-s");

        // Debug console mode also mirrors every line to a log file so the
        // pipe behaviour can be inspected without console redirection.
        string logPath = Path.Combine(Path.GetTempPath(), "AdbTether_service.log");
        void Trace(string msg)
        {
            Console.WriteLine(msg);
            try { File.AppendAllText(logPath, $"{DateTime.Now:HH:mm:ss.fff} {msg}\n"); } catch { }
        }

        if (asService)
        {
            using var service = new EngineService(Trace);
            ServiceBase.Run(service);
            return 0;
        }

        Trace($"AdbTether Service [debug] log={logPath}");
        var cts = new CancellationTokenSource();
        try
        {
            int init = EngineBridge.Init();
            Trace($"top_init -> {init}");
            if (init != 0)
                return 2;
            var server = new TcpServer(cts.Token, Trace);
            Trace("Listening on 127.0.0.1:47012");
            Trace("Press Ctrl+C to stop.");
            Console.CancelKeyPress += (_, e) => { e.Cancel = true; cts.Cancel(); };
            Task.Run(() => server.Run(), cts.Token).Wait(cts.Token);
        }
        catch (OperationCanceledException)
        {
        }
        catch (Exception ex)
        {
            Trace($"FATAL: {ex}");
            return 1;
        }
        finally
        {
            EngineBridge.Cleanup();
        }
        return 0;
    }
}

