namespace AdbTether.Models;

/// <summary>
/// A phone as the shell shows it. The serial is what adb is addressed with, the
/// name is what the user recognises, which the phone reports itself.
/// </summary>
public sealed record AdbDevice(string Serial, string Name);
