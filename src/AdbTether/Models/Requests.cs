namespace AdbTether.Models;

/// <summary>Validated values returned by the TCP/IP connect dialog.</summary>
public sealed record AdbConnectRequest(string IpAddress, int Port);

/// <summary>Validated values returned by the pairing dialog.</summary>
public sealed record AdbPairRequest(string IpAddress, int Port, string PairingCode);
