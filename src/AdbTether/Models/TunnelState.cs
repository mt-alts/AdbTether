namespace AdbTether.Models;

/// <summary>Coarse lifecycle state of the tunnel, used to drive status visuals.</summary>
public enum TunnelState
{
    Idle,
    Working,
    Active,
    Error,
}

/// <summary>
/// Translates the engine's free-form status string into a <see cref="TunnelState"/>.
/// This is the single place that knows the wording, so adding a new status only
/// means editing this class (open/closed principle).
/// </summary>
public static class StatusClassifier
{
    public static TunnelState Classify(string status, bool tunnelActive)
    {
        if (IsWorking(status))
            return TunnelState.Working;
        if (IsError(status))
            return TunnelState.Error;
        if (tunnelActive || IsActive(status))
            return TunnelState.Active;
        return TunnelState.Idle;
    }

    private static bool IsWorking(string s) =>
        s.Contains("Connecting") || s.Contains("Checking") || s.Contains("Starting");

    private static bool IsError(string s) =>
        s.Contains("Error") || s.Contains("Offline") || s.Contains("No proxy") || s.Contains("not reachable");

    private static bool IsActive(string s) =>
        s.Contains("Connected") || s.Contains("Online") || s.Contains("active");

    /// <summary>
    /// True when the engine reported that the proxy connection dropped (and the
    /// tunnel was stopped as a result).
    /// </summary>
    public static bool IsConnectionLost(string status) =>
        status.Contains("connection lost");
}
