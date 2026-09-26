// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

/// Default trigger phrases. Matching is case-insensitive and requires a word boundary.
pub const DEFAULT_MENTIONS: &str = "/blora,/ba,@blora";

/// Split a comma-separated mention list. Empty pieces are dropped.
#[must_use]
pub fn parse_mention_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(part))
        {
            out.push(part.to_string());
        }
    }
    if out.is_empty() {
        out.extend(parse_mention_list(DEFAULT_MENTIONS));
    }
    out
}

/// Byte range of the first mention that stands alone in `body`.
#[must_use]
pub fn find_mention(body: &str, mentions: &[String]) -> Option<(usize, usize)> {
    let mut found: Option<(usize, usize)> = None;
    for mention in mentions {
        if let Some(span) = mention_span(body, mention) {
            found = Some(match found {
                Some(current) if current.0 <= span.0 => current,
                _ => span,
            });
        }
    }
    found
}

/// The comment is only a trigger, optionally followed by sentence punctuation.
#[must_use]
pub fn is_bare_mention(body: &str, mentions: &[String]) -> bool {
    let trimmed = body.trim().trim_end_matches(is_sentence_punctuation).trim();
    mentions
        .iter()
        .any(|mention| trimmed.eq_ignore_ascii_case(mention))
}

fn mention_span(body: &str, mention: &str) -> Option<(usize, usize)> {
    if mention.is_empty() || !mention.is_ascii() {
        return None;
    }
    let needle = mention.as_bytes();
    let bytes = body.as_bytes();
    if needle.len() > bytes.len() {
        return None;
    }
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        let end = index + needle.len();
        let matched = bytes[index..end].eq_ignore_ascii_case(needle);
        let aligned = body.is_char_boundary(index) && body.is_char_boundary(end);
        if matched && aligned {
            let before_ok = index == 0 || bytes[index - 1].is_ascii_whitespace();
            let after_ok = end == bytes.len() || is_boundary_after(&body[end..]);
            if before_ok && after_ok {
                return Some((index, end));
            }
        }
        index += 1;
    }
    None
}

fn is_boundary_after(rest: &str) -> bool {
    let Some(ch) = rest.chars().next() else {
        return true;
    };
    ch.is_whitespace() || is_sentence_punctuation(ch)
}

fn is_sentence_punctuation(ch: char) -> bool {
    matches!(
        ch,
        ',' | '.'
            | '!'
            | '?'
            | ';'
            | ':'
            | ')'
            | ']'
            | '}'
            | '"'
            | '\''
            | '，'
            | '。'
            | '！'
            | '？'
            | '、'
            | '：'
            | '；'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mentions() -> Vec<String> {
        parse_mention_list(DEFAULT_MENTIONS)
    }

    #[test]
    fn matches_slash_at_and_short_form() {
        let mentions = mentions();
        assert!(find_mention("/blora 总结", &mentions).is_some());
        assert!(find_mention("请 @Blora 看一下", &mentions).is_some());
        assert!(find_mention("/BA 修好", &mentions).is_some());
        assert!(find_mention("/blora", &mentions).is_some());
    }

    #[test]
    fn rejects_partial_words_and_emails() {
        let mentions = mentions();
        assert!(find_mention("see /blora-docs", &mentions).is_none());
        assert!(find_mention("see /bar", &mentions).is_none());
        assert!(find_mention("mail a@blora.com", &mentions).is_none());
        assert!(find_mention("prefix/blora suffix", &mentions).is_none());
    }

    #[test]
    fn bare_mention_ignores_case_and_punctuation() {
        let mentions = mentions();
        assert!(is_bare_mention("  @blora  ", &mentions));
        assert!(is_bare_mention("/Blora。", &mentions));
        assert!(!is_bare_mention("@blora 看一下", &mentions));
    }
}
