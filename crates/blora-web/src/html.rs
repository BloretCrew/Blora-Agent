// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Small, dependency-free HTML to text conversion. It keeps headings, list
//! items, paragraphs, and absolute links, and drops scripts, styles, and markup.

/// Elements whose content is never shown.
const SKIP: &[&str] = &[
    "script", "style", "noscript", "svg", "template", "iframe", "canvas", "head",
];
/// Elements that start a new line.
const BLOCK: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "main",
    "header",
    "footer",
    "nav",
    "aside",
    "br",
    "hr",
    "tr",
    "table",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "pre",
    "blockquote",
    "figure",
    "figcaption",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
];

/// Convert an HTML document to plain text. The `<title>` becomes the first line.
#[must_use]
pub fn html_to_text(html: &str) -> String {
    let title = extract_title(html);
    let mut out = String::new();
    if let Some(title) = &title {
        out.push_str("# ");
        out.push_str(title);
        out.push_str("\n\n");
    }
    let mut skip_depth: Vec<String> = Vec::new();
    let mut link: Option<String> = None;
    let mut link_text = String::new();
    let mut pre = 0usize;
    let mut rest = html;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            push_text(
                &mut out,
                &mut link_text,
                link.is_some(),
                rest,
                pre > 0,
                &skip_depth,
            );
            break;
        };
        push_text(
            &mut out,
            &mut link_text,
            link.is_some(),
            &rest[..lt],
            pre > 0,
            &skip_depth,
        );
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
            continue;
        }
        let Some(gt) = rest.find('>') else {
            break;
        };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let body = tag.trim_start_matches('/').trim();
        let name: String = body
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        if let Some(open) = skip_depth.last() {
            if closing && *open == name {
                skip_depth.pop();
            } else if !closing && SKIP.contains(&name.as_str()) && !body.ends_with('/') {
                skip_depth.push(name);
            }
            continue;
        }
        if !closing && SKIP.contains(&name.as_str()) && !body.ends_with('/') {
            skip_depth.push(name);
            continue;
        }
        match (name.as_str(), closing) {
            ("a", false) => {
                link = attribute(body, "href")
                    .filter(|href| href.starts_with("http://") || href.starts_with("https://"));
                link_text.clear();
            }
            ("a", true) => {
                let text = collapse(&link_text);
                match link.take() {
                    Some(href) if !text.is_empty() && text != href => {
                        out.push_str(&format!("[{text}]({href})"));
                    }
                    Some(href) => out.push_str(&href),
                    None => out.push_str(&text),
                }
                link_text.clear();
            }
            ("pre", false) => {
                pre += 1;
                newline(&mut out, 2);
            }
            ("pre", true) => {
                pre = pre.saturating_sub(1);
                newline(&mut out, 2);
            }
            (heading, false) if heading.len() == 2 && heading.starts_with('h') => {
                newline(&mut out, 2);
                let level = heading[1..].parse::<usize>().unwrap_or(1).clamp(1, 6);
                out.push_str(&"#".repeat(level));
                out.push(' ');
            }
            ("li", false) => {
                newline(&mut out, 1);
                out.push_str("- ");
            }
            ("td" | "th", false) => out.push_str(" | "),
            (block, _) if BLOCK.contains(&block) => {
                let single = matches!(block, "br" | "tr" | "li" | "dt" | "dd");
                newline(&mut out, if single { 1 } else { 2 });
            }
            _ => {}
        }
    }
    tidy(&out)
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let end = lower[open_end..].find("</title")? + open_end;
    let title = collapse(&decode_entities(&html[open_end..end]));
    (!title.is_empty()).then_some(title)
}

fn push_text(
    out: &mut String,
    link_text: &mut String,
    in_link: bool,
    raw: &str,
    preformatted: bool,
    skip: &[String],
) {
    if !skip.is_empty() || raw.is_empty() {
        return;
    }
    let decoded = decode_entities(raw);
    if in_link {
        link_text.push_str(&decoded);
        return;
    }
    if preformatted {
        out.push_str(&decoded);
        return;
    }
    let mut last_space = out.ends_with(|ch: char| ch.is_whitespace()) || out.is_empty();
    for ch in decoded.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
}

fn newline(out: &mut String, count: usize) {
    while out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        return;
    }
    let existing = out.chars().rev().take_while(|ch| *ch == '\n').count();
    for _ in existing..count {
        out.push('\n');
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trim each line and collapse runs of blank lines.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(found) = lower[search_from..].find(name) {
        let at = search_from + found;
        search_from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = lower[at + name.len()..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let value = tag[value_start..].trim_start();
        let parsed = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or(""),
            _ => value
                .split(|ch: char| ch.is_whitespace() || ch == '>')
                .next()
                .unwrap_or(""),
        };
        return Some(decode_entities(parsed));
    }
    None
}

/// Decode the named entities that matter for reading plus numeric references.
#[must_use]
pub(crate) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "copy" => Some('©'),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &rest[semi + 1..];
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_structure_and_drops_scripts() {
        let html = r#"<html><head><title>Docs &amp; Notes</title><style>p{}</style></head>
            <body><script>alert("x")</script><nav>Menu</nav>
            <h2>Install</h2><p>Run   the <b>installer</b>.</p>
            <ul><li>One</li><li>Two &lt;3</li></ul>
            <p>See <a href="https://example.com/a">the guide</a> and <a href="/rel">local</a>.</p>
            <pre>line 1
  line 2</pre></body></html>"#;
        let text = html_to_text(html);
        assert!(text.starts_with("# Docs & Notes"), "{text}");
        assert!(text.contains("## Install"));
        assert!(text.contains("Run the installer."));
        assert!(text.contains("- One\n- Two <3"), "{text}");
        assert!(text.contains("[the guide](https://example.com/a)"));
        assert!(text.contains("local."));
        assert!(text.contains("line 1\n  line 2"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("p{}"));
    }

    #[test]
    fn decodes_numeric_entities() {
        assert_eq!(
            decode_entities("&#20320;&#x597D; &amp; &bogus;"),
            "你好 & &bogus;"
        );
    }
}

#[cfg(test)]
mod spacing_tests {
    use super::html_to_text;

    #[test]
    fn paragraphs_do_not_start_with_spaces() {
        let text = html_to_text("<div>\n   <p>\n  alpha </p>\n <p> beta</p></div>");
        assert_eq!(text, "alpha\n\nbeta");
    }
}
