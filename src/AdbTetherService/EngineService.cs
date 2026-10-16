using System.Diagnostics;
using System.ServiceProcess;

namespace AdbTetherService;

/// <summary>
/// Windows Service host. On start it initialises the engine (wintun + route
/// primitives are created in-process under SYSTEM) and opens the named-pipe
/// server for the UI. Debug mode (console) is used for development.
/// </summary>
internal sealed class EngineService : ServiceBase
{
    private CancellationTokenSource? _cts;
    private Task? _pipeTask;
    private readonly Action<string> _trace;

    public EngineService(Action<string>? trace = null)
    {
        ServiceName = "AdbTetherService";
        CanStop = true;
        CanPauseAndContinue = false;
        AutoLog = true;
        _trace = trace ?? (_ => { });
    }

    protected override void OnStart(string[] args)
    {
        _cts = new CancellationTokenSource();
        try
        {
            int init = EngineBridge.Init();
            if (init != 0)
                throw new InvalidOperationException($"top_init failed: {init}");
            var server = new TcpServer(_cts.Token, _trace);
            _pipeTask = Task.Run(() => server.Run(), _cts.Token);
        }
        catch (Exception ex)
        {
            EventLog.WriteEntry(ServiceName, $"Start failed: {ex}", EventLogEntryType.Error);
            throw;
        }
    }

    protected override void OnStop()
    {
        _cts?.Cancel();
        try { _pipeTask?.Wait(TimeSpan.FromSeconds(5)); } catch { }
        EngineBridge.Cleanup();
        _cts?.Dispose();
    }
}
