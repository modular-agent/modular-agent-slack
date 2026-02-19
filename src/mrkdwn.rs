use std::sync::LazyLock;

use regex::Regex;

struct Patterns {
    crlf: Regex,
    null_byte: Regex,
    fenced_code: Regex,
    inline_code: Regex,
    table: Regex,
    html_bold_b: Regex,
    html_bold_strong: Regex,
    html_italic_i: Regex,
    html_italic_em: Regex,
    html_strike_s: Regex,
    html_strike_del: Regex,
    html_strike_strike: Regex,
    html_code: Regex,
    html_pre: Regex,
    html_link: Regex,
    html_br: Regex,
    html_heading: Regex,
    html_li: Regex,
    html_p: Regex,
    html_hr: Regex,
    html_any_tag: Regex,
    html_entity_amp: Regex,
    html_entity_lt: Regex,
    html_entity_gt: Regex,
    html_entity_quot: Regex,
    html_entity_apos: Regex,
    md_image: Regex,
    md_link: Regex,
    md_bold_italic: Regex,
    md_bold: Regex,
    md_italic: Regex,
    md_strikethrough: Regex,
    md_heading: Regex,
    md_ul_dash: Regex,
    md_ul_star: Regex,
    md_hr: Regex,
    excess_newlines: Regex,
}

static RE: LazyLock<Patterns> = LazyLock::new(|| {
    Patterns {
    crlf: Regex::new(r"\r\n").unwrap(),
    null_byte: Regex::new(r"\x00").unwrap(),
    fenced_code: Regex::new(r"(?s)```[^\n]*\n(.*?)```").unwrap(),
    inline_code: Regex::new(r"`([^`\n]+)`").unwrap(),
    table: Regex::new(r"(?m)((?:^[ \t]*\|.+\|[ \t]*\n)+^[ \t]*\|[\s:]*-[\s:\-|]*\|[ \t]*\n(?:^[ \t]*\|.+\|[ \t]*\n?)*)").unwrap(),
    html_bold_b: Regex::new(r"(?si)<b>(.*?)</b>").unwrap(),
    html_bold_strong: Regex::new(r"(?si)<strong>(.*?)</strong>").unwrap(),
    html_italic_i: Regex::new(r"(?si)<i>(.*?)</i>").unwrap(),
    html_italic_em: Regex::new(r"(?si)<em>(.*?)</em>").unwrap(),
    html_strike_s: Regex::new(r"(?si)<s>(.*?)</s>").unwrap(),
    html_strike_del: Regex::new(r"(?si)<del>(.*?)</del>").unwrap(),
    html_strike_strike: Regex::new(r"(?si)<strike>(.*?)</strike>").unwrap(),
    html_code: Regex::new(r"(?si)<code>(.*?)</code>").unwrap(),
    html_pre: Regex::new(r"(?si)<pre>(.*?)</pre>").unwrap(),
    html_link: Regex::new(r#"(?si)<a\s[^>]*href=["']([^"']*)["'][^>]*>(.*?)</a>"#).unwrap(),
    html_br: Regex::new(r"(?i)<br\s*/?>").unwrap(),
    html_heading: Regex::new(r"(?si)<h[1-6][^>]*>(.*?)</h[1-6]>").unwrap(),
    html_li: Regex::new(r"(?si)<li[^>]*>(.*?)</li>").unwrap(),
    html_p: Regex::new(r"(?si)</?p[^>]*>").unwrap(),
    html_hr: Regex::new(r"(?i)<hr\s*/?>").unwrap(),
    html_any_tag: Regex::new(r"<[^>]+>").unwrap(),
    html_entity_amp: Regex::new(r"&amp;").unwrap(),
    html_entity_lt: Regex::new(r"&lt;").unwrap(),
    html_entity_gt: Regex::new(r"&gt;").unwrap(),
    html_entity_quot: Regex::new(r"&quot;").unwrap(),
    html_entity_apos: Regex::new(r"&#0?39;|&apos;").unwrap(),
    md_image: Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap(),
    md_link: Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap(),
    md_bold_italic: Regex::new(r"\*\*\*(.+?)\*\*\*").unwrap(),
    md_bold: Regex::new(r"\*\*(.+?)\*\*").unwrap(),
    md_italic: Regex::new(r"\*([^*\n]+?)\*").unwrap(),
    md_strikethrough: Regex::new(r"~~(.+?)~~").unwrap(),
    md_heading: Regex::new(r"(?m)^#{1,6}\s+(.+)$").unwrap(),
    md_ul_dash: Regex::new(r"(?m)^(\s*)- ").unwrap(),
    md_ul_star: Regex::new(r"(?m)^(\s*)\* ").unwrap(),
    md_hr: Regex::new(r"(?m)^[-*_]{3,}\s*$").unwrap(),
    excess_newlines: Regex::new(r"\n{3,}").unwrap(),
}
});

/// Convert Markdown/HTML text to Slack mrkdwn format.
pub fn md_to_mrkdwn(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    let mut placeholders: Vec<String> = Vec::new();

    // Step 1: Normalize line endings, strip null bytes
    let mut text = RE.crlf.replace_all(input, "\n").into_owned();
    text = RE.null_byte.replace_all(&text, "").into_owned();

    // Step 2: Protect fenced code blocks (strip language identifiers)
    text = RE
        .fenced_code
        .replace_all(&text, |caps: &regex::Captures| {
            let code_content = &caps[1];
            let idx = placeholders.len();
            placeholders.push(format!("```\n{}```", code_content));
            format!("\x00CB{}\x00", idx)
        })
        .into_owned();

    // Step 3: Protect inline code
    text = RE
        .inline_code
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("`{}`", &caps[1]));
            format!("\x00IC{}\x00", idx)
        })
        .into_owned();

    // Step 4: Detect Markdown tables → wrap in code block and protect
    text = RE
        .table
        .replace_all(&text, |caps: &regex::Captures| {
            let trimmed: String = caps[0]
                .lines()
                .map(|line| line.trim())
                .collect::<Vec<_>>()
                .join("\n");
            let trimmed = trimmed.trim_end_matches('\n');
            let idx = placeholders.len();
            placeholders.push(format!("```\n{}\n```", trimmed));
            format!("\x00TB{}\x00", idx)
        })
        .into_owned();

    // Step 5: HTML tag conversion
    // <pre> → code block (protect)
    text = RE
        .html_pre
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("```\n{}\n```", &caps[1]));
            format!("\x00CB{}\x00", idx)
        })
        .into_owned();

    // <code> → inline code (protect)
    text = RE
        .html_code
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("`{}`", &caps[1]));
            format!("\x00IC{}\x00", idx)
        })
        .into_owned();

    // HTML bold → Slack bold (protect from italic pass)
    // ZWS (\u{200B}) around markers for Slack mrkdwn word boundary (CJK support)
    // See: https://github.com/slackapi/node-slack-sdk/issues/1698
    text = RE
        .html_bold_strong
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("\u{200B}*{}*\u{200B}", &caps[1]));
            format!("\x00BD{}\x00", idx)
        })
        .into_owned();
    text = RE
        .html_bold_b
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("\u{200B}*{}*\u{200B}", &caps[1]));
            format!("\x00BD{}\x00", idx)
        })
        .into_owned();

    text = RE
        .html_italic_em
        .replace_all(&text, "\u{200B}_${1}_\u{200B}")
        .into_owned();
    text = RE
        .html_italic_i
        .replace_all(&text, "\u{200B}_${1}_\u{200B}")
        .into_owned();
    text = RE
        .html_strike_del
        .replace_all(&text, "\u{200B}~$1~\u{200B}")
        .into_owned();
    text = RE
        .html_strike_s
        .replace_all(&text, "\u{200B}~$1~\u{200B}")
        .into_owned();
    text = RE
        .html_strike_strike
        .replace_all(&text, "\u{200B}~$1~\u{200B}")
        .into_owned();

    // <a href="url">text</a> → <url|text> (protect)
    text = RE
        .html_link
        .replace_all(&text, |caps: &regex::Captures| {
            let url = &caps[1];
            let link_text = strip_angle_brackets(&caps[2]);
            let idx = placeholders.len();
            placeholders.push(format!("<{}|{}>", url, link_text));
            format!("\x00LK{}\x00", idx)
        })
        .into_owned();

    text = RE.html_br.replace_all(&text, "\n").into_owned();
    text = RE
        .html_heading
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("\u{200B}*{}*\u{200B}", &caps[1]));
            format!("\n\x00BD{}\x00\n", idx)
        })
        .into_owned();
    text = RE.html_li.replace_all(&text, "\u{2022} $1\n").into_owned();
    text = RE.html_p.replace_all(&text, "\n").into_owned();
    text = RE.html_hr.replace_all(&text, "").into_owned();

    // Step 6: Markdown image/link BEFORE stripping remaining HTML tags
    // (link text may contain angle brackets like [click <here>](url))
    text = RE.md_image.replace_all(&text, "$2").into_owned();
    text = RE
        .md_link
        .replace_all(&text, |caps: &regex::Captures| {
            let link_text = strip_angle_brackets(&caps[1]);
            let url = &caps[2];
            let idx = placeholders.len();
            placeholders.push(format!("<{}|{}>", url, link_text));
            format!("\x00LK{}\x00", idx)
        })
        .into_owned();

    // Strip remaining HTML tags
    text = RE.html_any_tag.replace_all(&text, "").into_owned();

    // Step 7: HTML entity decode
    text = RE.html_entity_lt.replace_all(&text, "<").into_owned();
    text = RE.html_entity_gt.replace_all(&text, ">").into_owned();
    text = RE.html_entity_quot.replace_all(&text, "\"").into_owned();
    text = RE.html_entity_apos.replace_all(&text, "'").into_owned();
    text = RE.html_entity_amp.replace_all(&text, "&").into_owned();

    // Step 9: Bold/Italic conversion (order matters)
    // 9a: ***bold italic*** → *_bold italic_* → protect from italic pass
    text = RE
        .md_bold_italic
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("\u{200B}*_{}_*\u{200B}", &caps[1]));
            format!("\x00BI{}\x00", idx)
        })
        .into_owned();

    // 9b: **bold** → convert inner italic first, then protect as *content*
    text = RE
        .md_bold
        .replace_all(&text, |caps: &regex::Captures| {
            let inner = RE.md_italic.replace_all(&caps[1], "\u{200B}_${1}_\u{200B}");
            let idx = placeholders.len();
            placeholders.push(format!("\u{200B}*{}*\u{200B}", inner));
            format!("\x00BD{}\x00", idx)
        })
        .into_owned();

    // 9c: *italic* → _italic_ (bold/bold-italic already placeholder'd)
    text = RE
        .md_italic
        .replace_all(&text, "\u{200B}_${1}_\u{200B}")
        .into_owned();

    // Step 10: Strikethrough
    text = RE
        .md_strikethrough
        .replace_all(&text, "\u{200B}~$1~\u{200B}")
        .into_owned();

    // Step 11: Headings # text → *text* (protect from italic pass)
    text = RE
        .md_heading
        .replace_all(&text, |caps: &regex::Captures| {
            let idx = placeholders.len();
            placeholders.push(format!("\u{200B}*{}*\u{200B}", &caps[1]));
            format!("\x00BD{}\x00", idx)
        })
        .into_owned();

    // Step 12: Unordered lists
    text = RE.md_ul_dash.replace_all(&text, "$1\u{2022} ").into_owned();
    text = RE.md_ul_star.replace_all(&text, "$1\u{2022} ").into_owned();

    // Step 13: Horizontal rules → remove
    text = RE.md_hr.replace_all(&text, "").into_owned();

    // Step 14: Collapse excess newlines
    text = RE.excess_newlines.replace_all(&text, "\n\n").into_owned();

    // Step 15: Restore all placeholders
    for (idx, replacement) in placeholders.iter().enumerate().rev() {
        for prefix in &["CB", "IC", "TB", "LK", "BI", "BD"] {
            let token = format!("\x00{}{}\x00", prefix, idx);
            if text.contains(&token) {
                text = text.replace(&token, replacement);
                break;
            }
        }
    }

    // Collapse consecutive zero-width spaces
    while text.contains("\u{200B}\u{200B}") {
        text = text.replace("\u{200B}\u{200B}", "\u{200B}");
    }

    // Clean up zero-width spaces adjacent to whitespace (already a natural boundary)
    text = text.replace(" \u{200B}", " ");
    text = text.replace("\u{200B} ", " ");
    text = text.replace("\n\u{200B}", "\n");
    text = text.replace("\u{200B}\n", "\n");

    // Safety: strip any residual null bytes
    text = text.replace('\x00', "");

    text.trim().trim_matches('\u{200B}').to_string()
}

fn strip_angle_brackets(s: &str) -> String {
    s.replace(['<', '>'], "")
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Basic formatting ===

    #[test]
    fn test_bold() {
        assert_eq!(md_to_mrkdwn("**hello**"), "*hello*");
    }

    #[test]
    fn test_italic() {
        assert_eq!(md_to_mrkdwn("*hello*"), "_hello_");
    }

    #[test]
    fn test_bold_italic() {
        assert_eq!(md_to_mrkdwn("***hello***"), "*_hello_*");
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(md_to_mrkdwn("~~hello~~"), "~hello~");
    }

    // === Code ===

    #[test]
    fn test_inline_code_preserved() {
        assert_eq!(md_to_mrkdwn("use `**raw**` here"), "use `**raw**` here");
    }

    #[test]
    fn test_fenced_code_block_preserved() {
        let input = "text **bold**\n```\n**not bold**\n```\nmore **bold**";
        let output = md_to_mrkdwn(input);
        assert!(output.contains("*bold*"));
        assert!(output.contains("```\n**not bold**\n```"));
    }

    #[test]
    fn test_code_block_language_stripped() {
        let input = "```python\ndef hello():\n    pass\n```";
        let output = md_to_mrkdwn(input);
        assert!(output.starts_with("```\n"));
        assert!(output.contains("def hello():"));
        assert!(!output.contains("python"));
    }

    // === Links ===

    #[test]
    fn test_link() {
        assert_eq!(
            md_to_mrkdwn("[click](https://example.com)"),
            "<https://example.com|click>"
        );
    }

    #[test]
    fn test_image() {
        assert_eq!(
            md_to_mrkdwn("![alt](https://example.com/img.png)"),
            "https://example.com/img.png"
        );
    }

    #[test]
    fn test_link_with_angle_brackets_in_text() {
        assert_eq!(
            md_to_mrkdwn("[click <here>](https://example.com)"),
            "<https://example.com|click here>"
        );
    }

    // === Headers ===

    #[test]
    fn test_h1() {
        assert_eq!(md_to_mrkdwn("# Hello"), "*Hello*");
    }

    #[test]
    fn test_h2() {
        assert_eq!(md_to_mrkdwn("## World"), "*World*");
    }

    #[test]
    fn test_h3() {
        assert_eq!(md_to_mrkdwn("### Deep"), "*Deep*");
    }

    // === Lists ===

    #[test]
    fn test_unordered_list_dash() {
        assert_eq!(
            md_to_mrkdwn("- item 1\n- item 2"),
            "\u{2022} item 1\n\u{2022} item 2"
        );
    }

    #[test]
    fn test_unordered_list_star() {
        assert_eq!(
            md_to_mrkdwn("* item 1\n* item 2"),
            "\u{2022} item 1\n\u{2022} item 2"
        );
    }

    #[test]
    fn test_ordered_list_passthrough() {
        assert_eq!(md_to_mrkdwn("1. first\n2. second"), "1. first\n2. second");
    }

    // === Blockquote ===

    #[test]
    fn test_blockquote_passthrough() {
        assert_eq!(md_to_mrkdwn("> quoted text"), "> quoted text");
    }

    // === Horizontal rule ===

    #[test]
    fn test_hr_removed() {
        assert_eq!(md_to_mrkdwn("above\n---\nbelow"), "above\n\nbelow");
    }

    // === Tables ===

    #[test]
    fn test_table_wrapped_in_code_block() {
        let input = "| A | B |\n|---|---|\n| 1 | 2 |";
        let output = md_to_mrkdwn(input);
        assert!(output.contains("```\n| A | B |"));
        assert!(output.contains("| 1 | 2 |\n```"));
    }

    // === HTML tags ===

    #[test]
    fn test_html_bold() {
        assert_eq!(md_to_mrkdwn("<strong>hello</strong>"), "*hello*");
    }

    #[test]
    fn test_html_b() {
        assert_eq!(md_to_mrkdwn("<b>hello</b>"), "*hello*");
    }

    #[test]
    fn test_html_italic() {
        assert_eq!(md_to_mrkdwn("<em>hello</em>"), "_hello_");
    }

    #[test]
    fn test_html_strike() {
        assert_eq!(md_to_mrkdwn("<del>hello</del>"), "~hello~");
    }

    #[test]
    fn test_html_link() {
        assert_eq!(
            md_to_mrkdwn(r#"<a href="https://example.com">click</a>"#),
            "<https://example.com|click>"
        );
    }

    #[test]
    fn test_html_br() {
        assert_eq!(md_to_mrkdwn("hello<br>world"), "hello\nworld");
    }

    #[test]
    fn test_html_br_self_closing() {
        assert_eq!(md_to_mrkdwn("hello<br/>world"), "hello\nworld");
    }

    #[test]
    fn test_html_code() {
        assert_eq!(md_to_mrkdwn("<code>foo</code>"), "`foo`");
    }

    #[test]
    fn test_html_pre() {
        let output = md_to_mrkdwn("<pre>some code</pre>");
        assert!(output.contains("```\nsome code\n```"));
    }

    #[test]
    fn test_html_heading() {
        assert_eq!(md_to_mrkdwn("<h1>Title</h1>").trim(), "*Title*");
    }

    #[test]
    fn test_html_li() {
        assert_eq!(
            md_to_mrkdwn("<li>first</li><li>second</li>").trim(),
            "\u{2022} first\n\u{2022} second"
        );
    }

    #[test]
    fn test_html_tags_stripped() {
        assert_eq!(md_to_mrkdwn("<div><span>hello</span></div>"), "hello");
    }

    // === HTML entities ===

    #[test]
    fn test_html_entities() {
        assert_eq!(md_to_mrkdwn("a &amp; b &lt; c &gt; d"), "a & b < c > d");
    }

    #[test]
    fn test_html_entity_quot() {
        assert_eq!(md_to_mrkdwn("&quot;hello&quot;"), "\"hello\"");
    }

    // === Edge cases ===

    #[test]
    fn test_empty_input() {
        assert_eq!(md_to_mrkdwn(""), "");
    }

    #[test]
    fn test_plain_text_unchanged() {
        assert_eq!(md_to_mrkdwn("hello world"), "hello world");
    }

    #[test]
    fn test_crlf_normalized() {
        assert_eq!(md_to_mrkdwn("hello\r\nworld"), "hello\nworld");
    }

    #[test]
    fn test_excess_newlines_collapsed() {
        assert_eq!(md_to_mrkdwn("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn test_nested_bold_italic() {
        assert_eq!(
            md_to_mrkdwn("**bold and *italic* inside**"),
            "*bold and _italic_ inside*"
        );
    }

    // === Realistic LLM output ===

    #[test]
    fn test_llm_output() {
        let input = r#"Here's a summary:

## Key Points

1. **First point**: This is important
2. **Second point**: Also relevant

- Use `code` for examples
- Check [the docs](https://docs.example.com)

```python
def hello():
    print("**not converted**")
```

> Note: This is a blockquote"#;

        let output = md_to_mrkdwn(input);
        assert!(output.contains("*Key Points*"));
        assert!(output.contains("*First point*"));
        assert!(output.contains("<https://docs.example.com|the docs>"));
        assert!(output.contains("```\ndef hello():\n    print(\"**not converted**\")\n```"));
        assert!(output.contains("> Note: This is a blockquote"));
        assert!(output.contains("`code`"));
    }

    // === Zero-width space for Slack mrkdwn word boundary ===

    #[test]
    fn test_bold_adjacent_to_japanese() {
        let output = md_to_mrkdwn("**シンプル**な構成");
        // Leading ZWS trimmed at start of string; trailing ZWS before な provides boundary
        assert!(output.contains("*シンプル*\u{200B}"));
        assert!(!output.contains("**"));
    }

    #[test]
    fn test_italic_adjacent_to_text() {
        let output = md_to_mrkdwn("*重要*です");
        assert!(output.contains("_重要_\u{200B}"));
    }

    #[test]
    fn test_strikethrough_adjacent_to_japanese() {
        let output = md_to_mrkdwn("~~削除~~された");
        assert!(output.contains("~削除~\u{200B}"));
    }

    #[test]
    fn test_bold_with_space_no_extra_zwsp() {
        let output = md_to_mrkdwn("**bold** text");
        assert!(output.contains("*bold* text"));
    }

    #[test]
    fn test_html_bold_adjacent_to_japanese() {
        let output = md_to_mrkdwn("<strong>太字</strong>テスト");
        assert!(output.contains("*太字*\u{200B}"));
    }

    #[test]
    fn test_mixed_latin_cjk_bold() {
        let output = md_to_mrkdwn("The **重要** item");
        assert!(output.contains("*重要*"));
        assert!(!output.contains("\u{200B} "));
    }

    #[test]
    fn test_consecutive_bold() {
        let output = md_to_mrkdwn("**a****b**");
        assert!(output.contains("*a*"));
        assert!(output.contains("*b*"));
        assert!(!output.contains("\u{200B}\u{200B}"));
    }

    #[test]
    fn test_list_item_with_bold_japanese() {
        let output = md_to_mrkdwn("- **シンプル**な構成で学習しやすい");
        assert!(output.contains("• "));
        assert!(output.contains("*シンプル*"));
        assert!(!output.contains("**"));
    }

    // === Tables (trailing/leading whitespace) ===

    #[test]
    fn test_table_with_trailing_whitespace() {
        let input = "| A | B |  \n|---|---|\n| 1 | 2 |  \n";
        let output = md_to_mrkdwn(input);
        assert!(output.contains("```\n| A | B |"));
        assert!(output.contains("| 1 | 2 |"));
    }

    #[test]
    fn test_table_with_leading_whitespace() {
        let input = "  | A | B |\n  |---|---|\n  | 1 | 2 |";
        let output = md_to_mrkdwn(input);
        assert!(output.contains("```\n| A | B |"));
        assert!(output.contains("| 1 | 2 |"));
    }

    #[test]
    fn test_realistic_llm_table() {
        let input = "| Where to look | What you'll see |\n|---------------|----------------|\n| **Reuters** | Coverage of markets |\n| **Bloomberg** | Analysis on indices |";
        let output = md_to_mrkdwn(input);
        assert!(output.contains("```\n"));
        assert!(output.contains("| **Reuters**"));
    }

    #[test]
    fn test_full_llm_output_with_table() {
        let input = "Here's the info:\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n### Steps\n\n1. **First**: do this\n2. **Second**: do that\n\n- Use `code` here\n- Check [docs](https://example.com)";
        let output = md_to_mrkdwn(input);
        assert!(output.contains("```\n| A | B |"));
        assert!(output.contains("*Steps*"));
        assert!(output.contains("*First*"));
        assert!(output.contains("`code`"));
        assert!(output.contains("<https://example.com|docs>"));
    }

    #[test]
    fn test_mixed_html_and_markdown() {
        let input = "**bold** and <em>italic</em> with [link](https://example.com)";
        let output = md_to_mrkdwn(input);
        assert!(output.contains("*bold*"));
        assert!(output.contains("_italic_"));
        assert!(output.contains("<https://example.com|link>"));
    }

    #[test]
    fn test_unicode_content() {
        assert_eq!(md_to_mrkdwn("**太字**"), "*太字*");
        assert_eq!(md_to_mrkdwn("*斜体*"), "_斜体_");
    }

    #[test]
    fn test_emoji_in_bold() {
        assert_eq!(md_to_mrkdwn("**🎉 celebration 🎉**"), "*🎉 celebration 🎉*");
    }
}
