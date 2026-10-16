using Microsoft.UI.Xaml.Controls;
using AdbTether.Models;

namespace AdbTether.Pages;

public sealed partial class AboutPage : Page
{
    public AboutPage()
    {
        InitializeComponent();
        GitHubLink.NavigateUri = new Uri(AppInfo.RepositoryUrl);
    }
}
