//! Converts the small HTML subset Mastodon emits for post content into Pango
//! markup for `gtk::Label`, or into plain text.

use gtk::glib;

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let end = rest[..rest.len().min(12)].find(';');
        let decoded = end.and_then(|end| {
            let ent = &rest[1..end];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    u32::from_str_radix(&ent[2..], 16)
                        .ok()
                        .and_then(char::from_u32)
                }
                _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search = 0;
    while let Some(i) = lower[search..].find(name) {
        let i = search + i;
        search = i + name.len();
        let before_ok = i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace();
        let after = lower[search..].trim_start();
        if !before_ok || !after.starts_with('=') {
            continue;
        }
        let val_start = tag.len() - after.len() + 1;
        let v = tag[val_start..].trim_start();
        let (q, body) = match v.chars().next() {
            Some(q @ ('"' | '\'')) => (Some(q), &v[1..]),
            _ => (None, v),
        };
        let end = match q {
            Some(q) => body.find(q).unwrap_or(body.len()),
            None => body
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(body.len()),
        };
        return Some(decode_entities(&body[..end]));
    }
    None
}

enum Tok<'a> {
    Text(&'a str),
    /// (name, is_closing, raw tag body)
    Tag(String, bool, &'a str),
}

fn tokenize(html: &str) -> Vec<Tok<'_>> {
    let mut toks = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(0) => {
                let end = rest.find('>').unwrap_or(rest.len() - 1);
                let body = &rest[1..end];
                let closing = body.starts_with('/');
                let name = body
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                toks.push(Tok::Tag(name, closing, body));
                rest = &rest[end + 1..];
            }
            Some(n) => {
                toks.push(Tok::Text(&rest[..n]));
                rest = &rest[n..];
            }
            None => {
                toks.push(Tok::Text(rest));
                rest = "";
            }
        }
    }
    toks
}

/// Walks the HTML, calling `emit` with already-decoded text pieces and
/// structural events.
enum Ev<'a> {
    Text(&'a str),
    Newlines(usize),
    LinkStart(String),
    LinkEnd,
    Open(&'static str),
    Close(&'static str),
}

fn walk(html: &str, mut emit: impl FnMut(Ev<'_>)) {
    // Each open <span> records whether it hides its content.
    let mut spans: Vec<bool> = Vec::new();
    let mut hidden = 0usize;
    let mut started = false;
    let mut in_link = false;
    let mut pending_ellipsis = false;
    let mut list_depth = 0usize;
    for t in tokenize(html) {
        match t {
            Tok::Text(text) => {
                if hidden > 0 {
                    continue;
                }
                let text = decode_entities(text);
                if !text.is_empty() {
                    emit(Ev::Text(&text));
                    started = true;
                }
            }
            Tok::Tag(name, closing, raw) => match (name.as_str(), closing) {
                ("p", false) | ("blockquote", false) | ("pre", false) => {
                    if started {
                        emit(Ev::Newlines(2));
                    }
                    if name == "pre" {
                        emit(Ev::Open("tt"));
                    }
                }
                ("pre", true) => emit(Ev::Close("tt")),
                ("br", _) => emit(Ev::Newlines(1)),
                ("ul" | "ol", false) => list_depth += 1,
                ("ul" | "ol", true) => list_depth = list_depth.saturating_sub(1),
                ("li", false) => {
                    if started {
                        emit(Ev::Newlines(1));
                    }
                    emit(Ev::Text(&format!(
                        "{}• ",
                        "  ".repeat(list_depth.saturating_sub(1))
                    )));
                    started = true;
                }
                ("a", false) => {
                    if let Some(href) = attr(raw, "href") {
                        emit(Ev::LinkStart(href));
                        in_link = true;
                    }
                }
                ("a", true) => {
                    if in_link {
                        emit(Ev::LinkEnd);
                        in_link = false;
                    }
                }
                ("span", false) => {
                    let class = attr(raw, "class").unwrap_or_default();
                    let hide = class.split_whitespace().any(|c| c == "invisible");
                    if class.split_whitespace().any(|c| c == "ellipsis") {
                        pending_ellipsis = true;
                    }
                    spans.push(hide);
                    if hide {
                        hidden += 1;
                    }
                }
                ("span", true) => {
                    if spans.pop() == Some(true) {
                        hidden -= 1;
                    }
                    if pending_ellipsis {
                        emit(Ev::Text("…"));
                        pending_ellipsis = false;
                    }
                }
                ("b" | "strong", c) => emit(if c { Ev::Close("b") } else { Ev::Open("b") }),
                ("i" | "em", c) => emit(if c { Ev::Close("i") } else { Ev::Open("i") }),
                ("code", c) => emit(if c { Ev::Close("tt") } else { Ev::Open("tt") }),
                ("del" | "s", c) => emit(if c { Ev::Close("s") } else { Ev::Open("s") }),
                ("u", c) => emit(if c { Ev::Close("u") } else { Ev::Open("u") }),
                _ => {}
            },
        }
    }
    if in_link {
        emit(Ev::LinkEnd);
    }
}

/// Pango markup with `<a href>` links, suitable for `Label::set_markup`.
pub fn to_markup(html: &str) -> String {
    let mut out = String::new();
    let mut open: Vec<&'static str> = Vec::new();
    walk(html, |ev| match ev {
        Ev::Text(t) => out.push_str(&glib::markup_escape_text(t)),
        Ev::Newlines(n) => {
            // Collapse runs of breaks so that "</p><p>" doesn't triple-space.
            let have = out.chars().rev().take_while(|&c| c == '\n').count();
            for _ in have..n.max(have) {
                out.push('\n');
            }
        }
        Ev::LinkStart(href) => {
            out.push_str(&format!("<a href=\"{}\">", glib::markup_escape_text(&href)));
            open.push("a");
        }
        Ev::LinkEnd | Ev::Close(_) => {
            let want = match ev {
                Ev::Close(t) => t,
                _ => "a",
            };
            // Close in proper nesting order, reopening anything in between.
            if let Some(pos) = open.iter().rposition(|t| *t == want) {
                let reopen: Vec<_> = open.drain(pos..).collect();
                for t in reopen.iter().rev() {
                    out.push_str(&format!("</{t}>"));
                }
                for t in reopen.iter().skip(1) {
                    if *t != "a" {
                        out.push_str(&format!("<{t}>"));
                        open.push(t);
                    }
                }
            }
        }
        Ev::Open(t) => {
            out.push_str(&format!("<{t}>"));
            open.push(t);
        }
    });
    for t in open.iter().rev() {
        out.push_str(&format!("</{t}>"));
    }
    out.trim_end().to_string()
}

pub fn to_text(html: &str) -> String {
    let mut out = String::new();
    walk(html, |ev| match ev {
        Ev::Text(t) => out.push_str(t),
        Ev::Newlines(n) => {
            let have = out.chars().rev().take_while(|&c| c == '\n').count();
            for _ in have..n.max(have) {
                out.push('\n');
            }
        }
        _ => {}
    });
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mastodon_link() {
        let h = r#"<p>Hi <span class="h-card"><a href="https://x.y/@bob" class="u-url mention">@<span>bob</span></a></span> see <a href="https://example.com/very/long/path" rel="nofollow"><span class="invisible">https://</span><span class="ellipsis">example.com/very</span><span class="invisible">/long/path</span></a></p><p>a &amp; b<br>c</p>"#;
        assert_eq!(
            to_markup(h),
            "Hi <a href=\"https://x.y/@bob\">@bob</a> see <a href=\"https://example.com/very/long/path\">example.com/very…</a>\n\na &amp; b\nc"
        );
        assert_eq!(to_text(h), "Hi @bob see example.com/very…\n\na & b\nc");
    }

    #[test]
    fn misnested() {
        assert_eq!(to_markup("<b><i>x</b>y</i>"), "<b><i>x</i></b><i>y</i>");
    }
}
