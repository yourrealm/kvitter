//! Post markdown: rendering, tags, file references and plain-text excerpts.
//!
//! Raw HTML in a post is shown as text, never rendered. Links only keep http(s), mailto and
//! same-site paths. Only uploaded files render as images (the CSP blocks others anyway).

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd, html};

use crate::ids;

pub struct RenderOpts<'a> {
    /// Prefix for links the app generates (`/t/…`, `/files/…`): empty on pages, `PUBLIC_URL` in RSS.
    pub base: &'a str,
    /// Tag pages exist only for signed-in viewers; anonymous pages show tags as plain text.
    pub link_tags: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct Analysis {
    pub tags: Vec<String>,
    pub file_ids: Vec<String>,
    pub first_image: Option<String>,
}

fn parser(body: &str) -> Parser<'_> {
    Parser::new_ext(body, Options::ENABLE_STRIKETHROUGH)
}

pub fn render(body: &str, opts: &RenderOpts) -> String {
    let mut out_events: Vec<Event> = Vec::new();
    // For each open link or image: whether its Start was kept (so its End must be too), and
    // whether it was an image turned into a link.
    let mut open: Vec<Open> = Vec::new();
    let mut in_code_block = false;

    for event in parser(body) {
        match event {
            Event::Html(s) | Event::InlineHtml(s) => out_events.push(Event::Text(s)),
            Event::SoftBreak => out_events.push(Event::HardBreak),
            Event::Start(Tag::CodeBlock(k)) => {
                in_code_block = true;
                out_events.push(Event::Start(Tag::CodeBlock(k)));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out_events.push(Event::End(TagEnd::CodeBlock));
            }
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => match safe_href(&dest_url, opts.base) {
                Some(href) => {
                    open.push(Open::Kept);
                    out_events.push(Event::Start(Tag::Link { link_type, dest_url: href.into(), title, id }));
                }
                None => open.push(Open::Dropped),
            },
            Event::End(TagEnd::Link) => {
                if let Some(Open::Kept) = open.pop() {
                    out_events.push(Event::End(TagEnd::Link));
                }
            }
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                if let Some(file_id) = file_id(&dest_url) {
                    open.push(Open::Image);
                    out_events.push(Event::Start(Tag::Image {
                        link_type,
                        dest_url: format!("{}/files/{file_id}", opts.base).into(),
                        title,
                        id,
                    }));
                } else if let Some(href) = safe_href(&dest_url, opts.base) {
                    open.push(Open::ImageAsLink);
                    out_events.push(Event::Start(Tag::Link { link_type: LinkType::Inline, dest_url: href.into(), title, id }));
                } else {
                    open.push(Open::Dropped);
                }
            }
            Event::End(TagEnd::Image) => match open.pop() {
                Some(Open::Image) => out_events.push(Event::End(TagEnd::Image)),
                Some(Open::ImageAsLink) => out_events.push(Event::End(TagEnd::Link)),
                _ => {}
            },
            Event::Text(text) if !in_code_block && open.is_empty() => {
                for seg in segments(&text) {
                    match seg {
                        Seg::Text(t) => out_events.push(Event::Text(CowStr::from(t.to_string()))),
                        Seg::Url(u) => out_events.push(Event::Html(
                            format!("<a href=\"{0}\">{0}</a>", escape(u)).into(),
                        )),
                        Seg::Tag(raw) => {
                            let tag = raw.to_lowercase();
                            let html = if opts.link_tags {
                                format!(
                                    "<a class=\"tag\" href=\"{}/t/{}\">#{}</a>",
                                    opts.base,
                                    escape(&encode_path(&tag)),
                                    escape(raw)
                                )
                            } else {
                                format!("<span class=\"tag\">#{}</span>", escape(raw))
                            };
                            out_events.push(Event::Html(html.into()));
                        }
                    }
                }
            }
            other => out_events.push(other),
        }
    }

    let mut out = String::new();
    html::push_html(&mut out, out_events.into_iter());
    out
}

enum Open {
    Kept,
    Dropped,
    Image,
    ImageAsLink,
}

pub fn analyze(body: &str) -> Analysis {
    let mut a = Analysis::default();
    let mut depth = 0usize; // inside links/images, where `#` is not a tag
    let mut in_code_block = false;
    for event in parser(body) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Start(Tag::Link { dest_url, .. }) => {
                depth += 1;
                push_file(&mut a.file_ids, &dest_url);
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                depth += 1;
                if let Some(id) = file_id(&dest_url) {
                    a.first_image.get_or_insert_with(|| id.to_string());
                }
                push_file(&mut a.file_ids, &dest_url);
            }
            Event::End(TagEnd::Link | TagEnd::Image) => depth = depth.saturating_sub(1),
            Event::Text(text) if depth == 0 && !in_code_block => {
                for seg in segments(&text) {
                    if let Seg::Tag(raw) = seg {
                        let tag = raw.to_lowercase();
                        if !a.tags.contains(&tag) {
                            a.tags.push(tag);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    a
}

/// The post as plain text, whitespace collapsed, cut to about `max` characters.
pub fn excerpt(body: &str, max: usize) -> String {
    let mut text = String::new();
    for event in parser(body) {
        match event {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => text.push_str(&t),
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph | TagEnd::Item) => text.push(' '),
            _ => {}
        }
    }
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches(|c: char| c.is_ascii_punctuation()))
}

fn push_file(ids: &mut Vec<String>, url: &str) {
    if let Some(id) = file_id(url)
        && !ids.iter().any(|x| x == id)
    {
        ids.push(id.to_string());
    }
}

/// `/files/<id>` (what the composer inserts) → `<id>`.
fn file_id(url: &str) -> Option<&str> {
    url.strip_prefix("/files/").filter(|id| ids::is_valid(id))
}

fn safe_href(url: &str, base: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        Some(url.trim().to_string())
    } else if url.starts_with('/') && !url.starts_with("//") {
        Some(format!("{base}{url}"))
    } else if url.starts_with('#') {
        Some(url.to_string())
    } else {
        None
    }
}

#[derive(Debug, PartialEq)]
enum Seg<'a> {
    Text(&'a str),
    Url(&'a str),
    Tag(&'a str),
}

/// Split running text into plain text, bare URLs and `#tags`.
///
/// A tag is `#` followed by letters, digits, `_` or `-`, at the start of the text or after a
/// character that can't be part of a word, and containing at least one letter (so `#1` in
/// "issue #1" is not a tag).
fn segments(text: &str) -> Vec<Seg<'_>> {
    let mut out = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        let rest = &text[i..];
        let prev = text[..i].chars().next_back();
        let at_boundary = prev.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '&' || c == '/' || c == '#'));

        if at_boundary && (rest.starts_with("https://") || rest.starts_with("http://")) {
            let mut end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            while end > 0 && rest[..end].ends_with(['.', ',', ';', ':', '!', '?', ')', '\'', '"']) {
                end -= 1;
            }
            if rest[..end].len() > "https://".len() {
                if plain_start < i {
                    out.push(Seg::Text(&text[plain_start..i]));
                }
                out.push(Seg::Url(&rest[..end]));
                i += end;
                plain_start = i;
                continue;
            }
        }

        if bytes[i] == b'#' && at_boundary {
            let name_len: usize = rest[1..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .map(char::len_utf8)
                .sum();
            let name = rest[1..1 + name_len].trim_end_matches(['-', '_']);
            if name.chars().any(char::is_alphabetic) {
                if plain_start < i {
                    out.push(Seg::Text(&text[plain_start..i]));
                }
                out.push(Seg::Tag(name));
                i += 1 + name.len();
                plain_start = i;
                continue;
            }
        }

        i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
    }
    if plain_start < text.len() {
        out.push(Seg::Text(&text[plain_start..]));
    }
    out
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const PATH_SEGMENT: &percent_encoding::AsciiSet =
    &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.');

pub fn encode_path(segment: &str) -> String {
    percent_encoding::utf8_percent_encode(segment, PATH_SEGMENT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: RenderOpts = RenderOpts { base: "", link_tags: true };

    #[test]
    fn raw_html_is_escaped() {
        let html = render("hi <script>alert(1)</script> <b>x</b>", &PAGE);
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(!html.contains("<b>"), "{html}");

        let block = render("<div onclick=\"x()\">\nhello\n</div>", &PAGE);
        assert!(!block.contains("<div"), "{block}");
    }

    #[test]
    fn unsafe_links_are_dropped() {
        let html = render("[click](javascript:alert(1)) [ok](https://example.com)", &PAGE);
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains("click"), "{html}");
        assert!(html.contains("<a href=\"https://example.com\">ok</a>"), "{html}");
    }

    #[test]
    fn only_uploads_render_as_images() {
        let html = render("![lake](/files/k7mq3xzh) ![x](https://evil.example/t.png)", &PAGE);
        assert!(html.contains("<img src=\"/files/k7mq3xzh\" alt=\"lake\""), "{html}");
        assert!(html.contains("<a href=\"https://evil.example/t.png\">x</a>"), "{html}");
        let rss = render("![lake](/files/k7mq3xzh)", &RenderOpts { base: "https://kv.example", link_tags: false });
        assert!(rss.contains("src=\"https://kv.example/files/k7mq3xzh\""), "{rss}");
    }

    #[test]
    fn tags() {
        let body = "Sauna #Sauna and #rust-lang, issue #1, mail a#b, `#code` [#link](/x) #über_ ok";
        assert_eq!(analyze(body).tags, vec!["sauna", "rust-lang", "über"]);
        let html = render("#Sauna!", &PAGE);
        assert!(html.contains("<a class=\"tag\" href=\"/t/sauna\">#Sauna</a>!"), "{html}");
        let anon = render("#Sauna", &RenderOpts { base: "", link_tags: false });
        assert!(anon.contains("<span class=\"tag\">#Sauna</span>"), "{anon}");
        assert!(analyze("```\n#nope\n```").tags.is_empty());
        assert!(analyze("# Heading").tags.is_empty());
    }

    #[test]
    fn bare_urls_link_and_skip_fragments() {
        let html = render("see https://example.com/a#frag.", &PAGE);
        assert!(html.contains("<a href=\"https://example.com/a#frag\">https://example.com/a#frag</a>."), "{html}");
        assert!(analyze("https://example.com/#frag").tags.is_empty());
    }

    #[test]
    fn files_and_first_image() {
        let a = analyze("![a](/files/k7mq3xzh) [b](/files/23456789) ![c](/files/k7mq3xzh) ![bad](/files/nope)");
        assert_eq!(a.file_ids, vec!["k7mq3xzh", "23456789"]);
        assert_eq!(a.first_image.as_deref(), Some("k7mq3xzh"));
    }

    #[test]
    fn excerpts() {
        assert_eq!(excerpt("**Hello**  world\n\nsecond", 100), "Hello world second");
        assert_eq!(excerpt("one two three four", 9), "one two…");
    }

    #[test]
    fn newlines_are_line_breaks() {
        assert!(render("a\nb", &PAGE).contains("a<br />\nb"));
    }
}
