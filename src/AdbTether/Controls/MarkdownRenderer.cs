using System.Text;
using Markdig;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using MarkdigBlock = Markdig.Syntax.Block;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.System;
using Windows.UI;

namespace AdbTether.Controls;

/// <summary>
/// Renders the small Markdown subset used by the offline help file into WinUI
/// controls. No WebView or network access is involved.
/// </summary>
public static class MarkdownRenderer
{
    public static void Render(string markdown, Panel host)
    {
        host.Children.Clear();

        var document = Markdown.Parse(markdown);
        foreach (var block in document)
        {
            var element = RenderBlock(block);
            if (element is not null)
                host.Children.Add(element);
        }
    }

    private static FrameworkElement? RenderBlock(MarkdigBlock block)
    {
        switch (block)
        {
            case HeadingBlock heading:
                return RenderHeading(heading);

            case ParagraphBlock paragraph:
                return RenderParagraph(paragraph.Inline);

            case ListBlock list:
                return RenderList(list);

            case FencedCodeBlock fenced:
                return RenderCode(fenced.Lines.ToString());

            case CodeBlock code:
                return RenderCode(code.Lines.ToString());

            case QuoteBlock quote:
                return RenderQuote(quote);

            case ThematicBreakBlock:
                return new Rectangle
                {
                    Height = 1,
                    Margin = new Thickness(0, 8, 0, 8),
                    Fill = new SolidColorBrush(Color.FromArgb(60, 128, 128, 128)),
                };

            case HtmlBlock:
                return null;

            case ContainerBlock container:
            {
                var panel = new StackPanel { Spacing = 8 };
                foreach (var child in container)
                {
                    var element = RenderBlock(child);
                    if (element is not null)
                        panel.Children.Add(element);
                }
                return panel;
            }

            default:
                return null;
        }
    }

    private static TextBlock RenderHeading(HeadingBlock heading)
    {
        return new TextBlock
        {
            Text = PlainText(heading.Inline),
            TextWrapping = TextWrapping.Wrap,
            FontSize = heading.Level switch
            {
                1 => 24,
                2 => 20,
                3 => 17,
                _ => 15,
            },
            FontWeight = FontWeights.SemiBold,
            Margin = new Thickness(0, heading.Level == 1 ? 0 : 10, 0, 4),
            IsTextSelectionEnabled = true,
        };
    }

    private static RichTextBlock RenderParagraph(ContainerInline? inline)
    {
        var text = new RichTextBlock
        {
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
        };

        var paragraph = new Paragraph();
        AddInlines(paragraph.Inlines, inline);
        text.Blocks.Add(paragraph);
        return text;
    }

    private static StackPanel RenderList(ListBlock list)
    {
        var panel = new StackPanel { Spacing = 4, Margin = new Thickness(4, 0, 0, 4) };
        var index = 1;
        if (list.IsOrdered)
            int.TryParse(list.OrderedStart?.ToString(), out index);

        foreach (ListItemBlock item in list)
        {
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

            var marker = new TextBlock
            {
                Text = list.IsOrdered ? $"{index}." : "•",
                FontSize = 13,
                FontWeight = FontWeights.SemiBold,
                VerticalAlignment = VerticalAlignment.Top,
                Margin = new Thickness(0, 1, 0, 0),
            };
            Grid.SetColumn(marker, 0);
            row.Children.Add(marker);

            var content = new StackPanel { Spacing = 4 };
            for (var i = 0; i < item.Count; i++)
            {
                var element = RenderBlock(item[i]);
                if (element is not null)
                    content.Children.Add(element);
            }
            Grid.SetColumn(content, 1);
            row.Children.Add(content);

            panel.Children.Add(row);
            index++;
        }

        return panel;
    }

    private static Border RenderCode(string code)
    {
        var text = new TextBlock
        {
            Text = code.TrimEnd(),
            FontFamily = new FontFamily("Consolas"),
            FontSize = 12,
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
        };

        return new Border
        {
            Background = new SolidColorBrush(Color.FromArgb(18, 128, 128, 128)),
            CornerRadius = new CornerRadius(6),
            Padding = new Thickness(12),
            Margin = new Thickness(0, 2, 0, 6),
            Child = text,
        };
    }

    private static Border RenderQuote(QuoteBlock quote)
    {
        var content = new StackPanel { Spacing = 6 };
        foreach (var child in quote)
        {
            var element = RenderBlock(child);
            if (element is not null)
                content.Children.Add(element);
        }

        return new Border
        {
            BorderBrush = new SolidColorBrush(Color.FromArgb(120, 128, 128, 128)),
            BorderThickness = new Thickness(3, 0, 0, 0),
            Padding = new Thickness(12, 2, 0, 2),
            Margin = new Thickness(0, 4, 0, 6),
            Child = content,
        };
    }

    private static void AddInlines(InlineCollection target, ContainerInline? container)
    {
        if (container is null)
            return;

        foreach (var inline in container)
        {
            switch (inline)
            {
                case LiteralInline literal:
                    target.Add(new Run { Text = literal.Content.ToString() });
                    break;

                case CodeInline code:
                    target.Add(new Run
                    {
                        Text = code.Content,
                        FontFamily = new FontFamily("Consolas"),
                    });
                    break;

                case EmphasisInline emphasis:
                {
                    Span span = emphasis.DelimiterCount >= 2
                        ? new Bold()
                        : new Italic();

                    AddInlines(span.Inlines, emphasis);
                    target.Add(span);
                    break;
                }

                case LinkInline link:
                {
                    var text = PlainText(link);
                    if (Uri.TryCreate(link.Url, UriKind.Absolute, out var uri))
                    {
                        var hyperlink = new Hyperlink { NavigateUri = uri };
                        if (link.FirstChild is not null)
                            AddInlines(hyperlink.Inlines, link);
                        else
                            hyperlink.Inlines.Add(new Run { Text = text });

                        hyperlink.Click += (_, _) => _ = Launcher.LaunchUriAsync(uri);
                        target.Add(hyperlink);
                    }
                    else
                    {
                        target.Add(new Run { Text = text });
                    }
                    break;
                }

                case LineBreakInline:
                    target.Add(new LineBreak());
                    break;

                case ContainerInline nested:
                    AddInlines(target, nested);
                    break;
            }
        }
    }

    private static string PlainText(ContainerInline? container)
    {
        if (container is null)
            return string.Empty;

        var text = new StringBuilder();
        AppendPlainText(text, container);
        return text.ToString();
    }

    private static void AppendPlainText(StringBuilder text, ContainerInline container)
    {
        foreach (var inline in container)
        {
            switch (inline)
            {
                case LiteralInline literal:
                    text.Append(literal.Content.ToString());
                    break;

                case CodeInline code:
                    text.Append(code.Content);
                    break;

                case LineBreakInline:
                    text.Append(' ');
                    break;

                case ContainerInline nested:
                    AppendPlainText(text, nested);
                    break;
            }
        }
    }
}
