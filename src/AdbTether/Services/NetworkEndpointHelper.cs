using System.Net;
using System.Net.NetworkInformation;
using System.Net.Sockets;

namespace AdbTether.Services;

/// <summary>
/// Finds the active default-gateway address so ADB dialogs can prefill the
/// device IP (the phone is typically the gateway when tethering).
/// </summary>
public static class NetworkEndpointHelper
{
    public static string? GetDefaultGatewayAddress()
    {
        return NetworkInterface
            .GetAllNetworkInterfaces()
            .Where(x =>
                x.OperationalStatus == OperationalStatus.Up
                && x.NetworkInterfaceType != NetworkInterfaceType.Loopback
                && x.NetworkInterfaceType != NetworkInterfaceType.Tunnel
            )
            .SelectMany(x => x.GetIPProperties().GatewayAddresses)
            .Select(x => x.Address)
            .FirstOrDefault(x =>
                x.AddressFamily == AddressFamily.InterNetwork
                && !IPAddress.Any.Equals(x)
                && !IPAddress.None.Equals(x)
            )
            ?.ToString();
    }
}
