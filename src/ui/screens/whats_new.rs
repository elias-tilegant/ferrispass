//! "What's New" modal — shown once after a successful update restart and
//! re-openable from Settings. Renders the release notes through a tiny
//! Markdown-lite pass so GitHub-style bullets, headings and links survive.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{Sizable as _, h_flex, scroll::ScrollableElement as _, v_flex};

use crate::app::Overlay;
use crate::ui::app_shell::AppShell;
use crate::ui::palette;
use crate::update::UpdateInfo;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let info = match shell.state().read(cx).overlay() {
        Overlay::WhatsNew { info } => info.clone(),
        _ => return div().into_any_element(),
    };

    let notes_body = render_notes(&info, cx);

    div()
        .id("whats-new-backdrop")
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .bg(palette::transparent_overlay())
        .occlude()
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .child(
            v_flex()
                .id("whats-new-panel")
                .w(px(560.))
                .max_h(px(620.))
                .rounded(px(10.))
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border_strong())
                .shadow_lg()
                .overflow_hidden()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(header(&info, cx))
                .child(
                    v_flex()
                        .id("whats-new-body")
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .p_5()
                        .gap_1p5()
                        .child(notes_body),
                )
                .child(footer(cx)),
        )
        .into_any_element()
}

fn header(info: &UpdateInfo, cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .items_start()
        .justify_between()
        .gap_4()
        .p_5()
        .border_b_1()
        .border_color(palette::border())
        .child(
            v_flex()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_lg()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(palette::text())
                        .child(format!("What's new in FerrisPass {}", info.version)),
                )
                .when_some(info.pub_date.clone(), |this, date| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(palette::text_muted())
                            .child(date),
                    )
                }),
        )
        .child(close_icon(cx))
        .into_any_element()
}

fn close_icon(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("whats-new-close")
        .flex_shrink_0()
        .size(px(28.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .text_color(palette::text_muted())
        .hover(|s| s.bg(palette::sidebar()).text_color(palette::text()))
        .cursor_pointer()
        .child(
            gpui_component::Icon::from(gpui_component::IconName::Close)
                .with_size(gpui_component::Size::Size(px(14.))),
        )
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .into_any_element()
}

fn footer(cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .justify_end()
        .p_4()
        .border_t_1()
        .border_color(palette::border())
        .child(done_button(cx))
        .into_any_element()
}

fn done_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("whats-new-done")
        .h(px(30.))
        .px(px(14.))
        .rounded(px(6.))
        .bg(palette::blue())
        .text_color(palette::panel())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(palette::blue_hover()))
        .child("Done")
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .into_any_element()
}

fn render_notes(info: &UpdateInfo, cx: &mut Context<AppShell>) -> AnyElement {
    if info.notes.trim().is_empty() {
        return div()
            .text_sm()
            .text_color(palette::text_muted())
            .child("No release notes were included with this update.")
            .into_any_element();
    }

    let mut children: Vec<AnyElement> = Vec::new();
    let mut link_id = 0usize;
    for (idx, raw_line) in info.notes.lines().enumerate() {
        children.push(render_line(idx, raw_line, &mut link_id, cx));
    }
    v_flex().gap_1p5().children(children).into_any_element()
}

fn render_line(
    line_idx: usize,
    raw: &str,
    link_id: &mut usize,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (kind, content) = parse_line(raw);

    match kind {
        LineKind::Spacer => div().h(px(6.)).into_any_element(),
        LineKind::H1 => div()
            .pt_2()
            .text_base()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(palette::text())
            .child(inline_row(content, line_idx, link_id, cx))
            .into_any_element(),
        LineKind::H2 => div()
            .pt_1()
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(palette::text())
            .child(inline_row(content, line_idx, link_id, cx))
            .into_any_element(),
        LineKind::H3 => div()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(palette::text_muted())
            .child(inline_row(content, line_idx, link_id, cx))
            .into_any_element(),
        LineKind::Bullet => h_flex()
            .gap_2()
            .items_start()
            .child(
                div()
                    .flex_shrink_0()
                    .mt(px(8.))
                    .size(px(4.))
                    .rounded_full()
                    .bg(palette::text_muted()),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .line_height(px(20.))
                    .text_color(palette::text())
                    .child(inline_row(content, line_idx, link_id, cx)),
            )
            .into_any_element(),
        LineKind::Paragraph => div()
            .text_sm()
            .line_height(px(20.))
            .text_color(palette::text())
            .child(inline_row(content, line_idx, link_id, cx))
            .into_any_element(),
    }
}

fn inline_row(
    text: &str,
    line_idx: usize,
    link_id: &mut usize,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let segments = parse_inline(text);
    if segments.len() == 1 {
        return render_segment(segments.into_iter().next().unwrap(), line_idx, link_id, cx);
    }
    h_flex()
        .flex_wrap()
        .children(
            segments
                .into_iter()
                .map(|seg| render_segment(seg, line_idx, link_id, cx)),
        )
        .into_any_element()
}

fn render_segment(
    seg: InlineSegment,
    line_idx: usize,
    link_id: &mut usize,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    match seg {
        InlineSegment::Text(t) => div().child(t).into_any_element(),
        InlineSegment::Strong(t) => div()
            .font_weight(gpui::FontWeight::BOLD)
            .child(t)
            .into_any_element(),
        InlineSegment::Emphasis(t) => div().italic().child(t).into_any_element(),
        InlineSegment::Code(t) => div()
            .font_family("JetBrains Mono")
            .px(px(4.))
            .rounded(px(4.))
            .bg(palette::sidebar())
            .text_color(palette::text())
            .child(t)
            .into_any_element(),
        InlineSegment::Link { label, url } => link_span(label, url, line_idx, link_id, cx),
    }
}

fn link_span(
    label: String,
    url: String,
    line_idx: usize,
    link_id: &mut usize,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let id: SharedString = format!("whats-new-link-{line_idx}-{}", *link_id).into();
    *link_id += 1;
    let url_for_click = url.clone();
    div()
        .id(id)
        .text_color(palette::blue())
        .hover(|s| s.text_color(palette::blue_hover()))
        .cursor_pointer()
        .child(label)
        .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
            cx.open_url(&url_for_click);
        }))
        .into_any_element()
}

#[derive(Debug)]
enum LineKind {
    Spacer,
    H1,
    H2,
    H3,
    Bullet,
    Paragraph,
}

fn parse_line(raw: &str) -> (LineKind, &str) {
    let trimmed = raw.trim_end();
    if trimmed.trim().is_empty() {
        return (LineKind::Spacer, "");
    }
    if let Some(rest) = trimmed.strip_prefix("### ") {
        return (LineKind::H3, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("## ") {
        return (LineKind::H2, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("# ") {
        return (LineKind::H1, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return (LineKind::Bullet, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("* ") {
        return (LineKind::Bullet, rest);
    }
    (LineKind::Paragraph, trimmed)
}

#[derive(Debug)]
enum InlineSegment {
    Text(String),
    Strong(String),
    Emphasis(String),
    Code(String),
    Link { label: String, url: String },
}

/// Parse a single line of GitHub-style Markdown into styled segments. Handles
/// `[label](url)` and bare `http(s)://...` links, `**bold**`/`__bold__`,
/// `*italic*`/`_italic_` and `` `code` ``. Anything that isn't a recognised
/// span survives as plain text, so unknown syntax stays human-readable.
fn parse_inline(text: &str) -> Vec<InlineSegment> {
    let mut out: Vec<InlineSegment> = Vec::new();
    let mut buf = String::new();
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        if ch == '['
            && let Some((label, url, consumed)) = match_md_link(rest)
        {
            flush(&mut out, &mut buf);
            out.push(InlineSegment::Link { label, url });
            rest = &rest[consumed..];
            continue;
        }
        if (ch == 'h' || ch == 'H')
            && let Some(consumed) = match_bare_url(rest)
        {
            flush(&mut out, &mut buf);
            let url = rest[..consumed].to_string();
            out.push(InlineSegment::Link {
                label: url.clone(),
                url,
            });
            rest = &rest[consumed..];
            continue;
        }
        if let Some((content, consumed)) = match_delim(rest, "`") {
            flush(&mut out, &mut buf);
            out.push(InlineSegment::Code(content));
            rest = &rest[consumed..];
            continue;
        }
        // Underscores only delimit at a word boundary so `snake_case` stays
        // intact, matching how GitHub renders release notes.
        let underscore_ok = !buf.chars().next_back().is_some_and(char::is_alphanumeric);
        let strong = match_delim(rest, "**")
            .or_else(|| underscore_ok.then(|| match_delim(rest, "__")).flatten());
        if let Some((content, consumed)) = strong {
            flush(&mut out, &mut buf);
            out.push(InlineSegment::Strong(content));
            rest = &rest[consumed..];
            continue;
        }
        let emphasis = match_delim(rest, "*")
            .or_else(|| underscore_ok.then(|| match_delim(rest, "_")).flatten());
        if let Some((content, consumed)) = emphasis {
            flush(&mut out, &mut buf);
            out.push(InlineSegment::Emphasis(content));
            rest = &rest[consumed..];
            continue;
        }
        buf.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    flush(&mut out, &mut buf);
    if out.is_empty() {
        out.push(InlineSegment::Text(String::new()));
    }
    out
}

fn flush(out: &mut Vec<InlineSegment>, buf: &mut String) {
    if !buf.is_empty() {
        out.push(InlineSegment::Text(std::mem::take(buf)));
    }
}

/// Match a `delim … delim` span (e.g. `**bold**`). Returns the inner content
/// and the byte length consumed, or `None` when the closing delimiter is
/// missing or the content would be empty.
fn match_delim(s: &str, delim: &str) -> Option<(String, usize)> {
    let inner = s.strip_prefix(delim)?;
    let close = inner.find(delim)?;
    if close == 0 {
        return None;
    }
    Some((inner[..close].to_string(), delim.len() + close + delim.len()))
}

fn match_md_link(s: &str) -> Option<(String, String, usize)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'[') {
        return None;
    }
    let close_label = bytes[1..].iter().position(|b| *b == b']')? + 1;
    if bytes.get(close_label + 1) != Some(&b'(') {
        return None;
    }
    let url_start = close_label + 2;
    let close_url = bytes[url_start..].iter().position(|b| *b == b')')? + url_start;
    let label = s[1..close_label].to_string();
    let url = s[url_start..close_url].to_string();
    Some((label, url, close_url + 1))
}

fn match_bare_url(s: &str) -> Option<usize> {
    let lower = s.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    let end = s
        .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | ',' | ';'))
        .unwrap_or(s.len());
    if end < 8 {
        return None;
    }
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings() {
        assert!(matches!(parse_line("# Hello").0, LineKind::H1));
        assert!(matches!(parse_line("## Sub").0, LineKind::H2));
        assert!(matches!(parse_line("### Tiny").0, LineKind::H3));
    }

    #[test]
    fn parses_bullets() {
        assert!(matches!(parse_line("- one").0, LineKind::Bullet));
        assert!(matches!(parse_line("* two").0, LineKind::Bullet));
    }

    #[test]
    fn blank_lines_are_spacers() {
        assert!(matches!(parse_line("").0, LineKind::Spacer));
        assert!(matches!(parse_line("   ").0, LineKind::Spacer));
    }

    #[test]
    fn parses_md_link() {
        let (label, url, consumed) = match_md_link("[Zed](https://zed.dev) and more").unwrap();
        assert_eq!(label, "Zed");
        assert_eq!(url, "https://zed.dev");
        assert_eq!(consumed, "[Zed](https://zed.dev)".len());
    }

    #[test]
    fn parses_bare_url_until_whitespace_or_punct() {
        assert_eq!(
            match_bare_url("https://example.com/foo, rest"),
            Some("https://example.com/foo".len())
        );
        assert_eq!(match_bare_url("ftp://nope"), None);
    }

    #[test]
    fn inline_splits_into_text_and_link() {
        let segs = parse_inline("See [docs](https://x.y) for details.");
        assert_eq!(segs.len(), 3);
        assert!(matches!(&segs[0], InlineSegment::Text(t) if t == "See "));
        assert!(matches!(&segs[1], InlineSegment::Link { url, .. } if url == "https://x.y"));
        assert!(matches!(&segs[2], InlineSegment::Text(t) if t == " for details."));
    }

    #[test]
    fn inline_picks_up_bare_url() {
        let segs = parse_inline("visit https://example.com for info");
        assert_eq!(segs.len(), 3);
        assert!(
            matches!(&segs[1], InlineSegment::Link { url, .. } if url == "https://example.com")
        );
    }

    #[test]
    fn inline_parses_bold() {
        let segs = parse_inline("**Full Changelog**:");
        assert_eq!(segs.len(), 2);
        assert!(matches!(&segs[0], InlineSegment::Strong(t) if t == "Full Changelog"));
        assert!(matches!(&segs[1], InlineSegment::Text(t) if t == ":"));
    }

    #[test]
    fn inline_parses_underscore_bold() {
        let segs = parse_inline("__loud__");
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], InlineSegment::Strong(t) if t == "loud"));
    }

    #[test]
    fn inline_parses_italic_and_code() {
        let segs = parse_inline("an *emphasised* `value` here");
        assert!(matches!(&segs[1], InlineSegment::Emphasis(t) if t == "emphasised"));
        assert!(matches!(&segs[3], InlineSegment::Code(t) if t == "value"));
    }

    #[test]
    fn bold_wins_over_italic() {
        let segs = parse_inline("**bold**");
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], InlineSegment::Strong(t) if t == "bold"));
    }

    #[test]
    fn unmatched_delimiter_stays_plain() {
        let segs = parse_inline("2 * 3 = 6");
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], InlineSegment::Text(t) if t == "2 * 3 = 6"));
    }

    #[test]
    fn snake_case_underscores_stay_literal() {
        let segs = parse_inline("call some_function_name now");
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], InlineSegment::Text(t) if t == "call some_function_name now"));
    }

    #[test]
    fn multibyte_text_is_not_corrupted() {
        let segs = parse_inline("Behoben: Größe geändert – ä ö ü ß");
        assert_eq!(segs.len(), 1);
        assert!(matches!(&segs[0], InlineSegment::Text(t) if t == "Behoben: Größe geändert – ä ö ü ß"));
    }
}
