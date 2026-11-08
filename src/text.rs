//! Shortening text for notification banners.

/// Cuts `text` to `max_chars` characters, marking the cut with an ellipsis.
pub fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

/// The first `n` characters of `text`, without an ellipsis.
pub fn prefix(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

/// The first sentence of a Markdown reply, without decoration, cut to `max_chars`.
pub fn first_sentence(markdown: &str, max_chars: usize) -> Option<String> {
    let mut in_code = false;
    let line = markdown.lines().map(str::trim).find(|line| {
        if line.starts_with("```") {
            in_code = !in_code;
            return false;
        }
        !in_code && !line.is_empty() && !line.chars().all(|c| "-*_=|#".contains(c))
    })?;

    let mut line = line.trim_start_matches(['#', '>', ' ']);
    for bullet in ["- ", "* ", "+ "] {
        line = line.strip_prefix(bullet).unwrap_or(line);
    }
    if let Some((num, rest)) = line.split_once(". ")
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
    {
        line = rest;
    }
    let plain = line.replace("**", "").replace("__", "").replace('`', "");
    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");

    let end = plain
        .char_indices()
        .find_map(|(i, c)| match c {
            '。' | '！' | '？' => Some(i + c.len_utf8()),
            '.' | '!' | '?' => {
                let next = plain[i + 1..].chars().next();
                (next.is_none() || next == Some(' ')).then_some(i + 1)
            }
            _ => None,
        })
        .unwrap_or(plain.len());
    let sentence = plain[..end].trim();
    (!sentence.is_empty()).then(|| truncate(sentence, max_chars))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_is_char_safe() {
        assert_eq!(truncate("あいうえお", 3), "あいう…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(prefix("日本語のセッションID", 8), "日本語のセッショ");
        assert_eq!(prefix("abc", 8), "abc");
    }

    #[test]
    fn first_sentences() {
        let f = |s: &str| first_sentence(s, 80);
        assert_eq!(
            f("フックの設定はうまく動いています。ホストのサーバーに届いています。").as_deref(),
            Some("フックの設定はうまく動いています。")
        );
        assert_eq!(
            f("**`a0e9654`** をコミットしました").as_deref(),
            Some("a0e9654 をコミットしました")
        );
        assert_eq!(
            f("- 1 つ目の項目。\n- 2 つ目").as_deref(),
            Some("1 つ目の項目。")
        );
        assert_eq!(f("1. Build it. 2. Ship").as_deref(), Some("Build it."));
        assert_eq!(f("# Done.\n\nAll tests pass.").as_deref(), Some("Done."));
        assert_eq!(f("```sh\nls\n```\nDone!").as_deref(), Some("Done!"));
        assert_eq!(
            f("---\n\nVersion 1.2 is out").as_deref(),
            Some("Version 1.2 is out")
        );
        assert_eq!(f("   \n\n"), None);
        assert_eq!(
            first_sentence(&"あ".repeat(200), 80)
                .unwrap()
                .chars()
                .count(),
            81
        );
    }
}
