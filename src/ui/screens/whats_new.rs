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
        return match segments.into_iter().next().unwrap() {
            InlineSegment::Text(t) => div().child(t).into_any_element(),
            InlineSegment::Link { label, url } => link_span(label, url, line_idx, link_id, cx),
        };
    }
    h_flex()
        .flex_wrap()
        .children(segments.into_iter().map(|seg| match seg {
            InlineSegment::Text(t) => div().child(t).into_any_element(),
            InlineSegment::Link { label, url } => link_span(label, url, line_idx, link_id, cx),
        }))
        .into_any_element()
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
    Link { label: String, url: String },
}

/// Parse `[label](url)` and bare `http(s)://...` URLs into segments. Bold,
/// italics and inline code are intentionally skipped — GitHub release notes
/// mostly need links + bullets, and a fuller Markdown pass would warrant a
/// crate dependency. Everything that isn't a recognised link survives as
/// plain text, so unknown syntax stays human-readable.
fn parse_inline(text: &str) -> Vec<InlineSegment> {
    let mut out: Vec<InlineSegment> = Vec::new();
    let mut buf = String::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some((label, url, consumed)) = match_md_link(&text[i..])
        {
            if !buf.is_empty() {
                out.push(InlineSegment::Text(std::mem::take(&mut buf)));
            }
            out.push(InlineSegment::Link { label, url });
            i += consumed;
            continue;
        }
        if (bytes[i] == b'h' || bytes[i] == b'H')
            && let Some(consumed) = match_bare_url(&text[i..])
        {
            if !buf.is_empty() {
                out.push(InlineSegment::Text(std::mem::take(&mut buf)));
            }
            let url = text[i..i + consumed].to_string();
            out.push(InlineSegment::Link {
                label: url.clone(),
                url,
            });
            i += consumed;
            continue;
        }
        buf.push(bytes[i] as char);
        i += 1;
    }
    if !buf.is_empty() {
        out.push(InlineSegment::Text(buf));
    }
    if out.is_empty() {
        out.push(InlineSegment::Text(String::new()));
    }
    out
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
}
