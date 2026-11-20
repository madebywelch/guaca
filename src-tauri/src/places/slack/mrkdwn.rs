pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        push_escaped(&mut out, c);
    }
    out
}

fn push_escaped(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(c),
    }
}

pub fn mrkdwn(markdown: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fenced = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if !fenced && trimmed.len() > 6 && trimmed.starts_with("```") && trimmed.ends_with("```") {
            out.push(format!("```{}```", escape(&trimmed[3..trimmed.len() - 3])));
            continue;
        }
        if trimmed.starts_with("```") {
            fenced = !fenced;
            out.push("```".to_string());
            continue;
        }
        out.push(if fenced { escape(line) } else { block(line) });
    }
    if fenced {
        out.push("```".to_string());
    }
    out.join("\n")
}

fn block(line: &str) -> String {
    let rest = line.trim_start();
    let indent = &line[..line.len() - rest.len()];
    if let Some(title) = heading(rest) {
        return format!("*{}*", inline(&title.replace("**", "").replace("__", "")));
    }
    if is_rule(rest) {
        return "---".to_string();
    }
    if let Some(item) = ["- ", "* ", "+ "].iter().find_map(|mark| rest.strip_prefix(mark)) {
        return format!("{indent}\u{2022} {}", inline(item));
    }
    if let Some(quote) = rest.strip_prefix("> ").or((rest == ">").then_some("")) {
        return format!("{indent}> {}", inline(quote));
    }
    format!("{indent}{}", inline(rest))
}

fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    line[hashes..].strip_prefix(' ').map(str::trim)
}

fn is_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3 && ['-', '*', '_'].iter().any(|mark| compact.chars().all(|c| c == *mark))
}

fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '`' {
            if let Some(end) = (at + 1..chars.len()).find(|j| chars[*j] == '`') {
                out.push('`');
                out.push_str(&escape(&collect(&chars, at + 1, end)));
                out.push('`');
                at = end + 1;
                continue;
            }
        }
        if c == '[' {
            if let Some((label, url, end)) = link(&chars, at) {
                out.push_str(&format!("<{}|{}>", address(&url), inline(&label)));
                at = end + 1;
                continue;
            }
        }
        if let Some((open, close)) = pair(&chars, at) {
            if let Some(end) = closing(&chars, at + open.len(), &open) {
                out.push_str(close);
                out.push_str(&inline(&collect(&chars, at + open.len(), end)));
                out.push_str(close);
                at = end + open.len();
                continue;
            }
        }
        push_escaped(&mut out, c);
        at += 1;
    }
    out
}

fn collect(chars: &[char], from: usize, to: usize) -> String {
    chars[from..to].iter().collect()
}

fn pair(chars: &[char], at: usize) -> Option<(Vec<char>, &'static str)> {
    let c = chars[at];
    let next = chars.get(at + 1).copied();
    match (c, next) {
        ('*', Some('*')) | ('_', Some('_')) => Some((vec![c, c], "*")),
        ('~', Some('~')) => Some((vec![c, c], "~")),
        ('*', _) | ('_', _) => Some((vec![c], "_")),
        _ => None,
    }
}

fn closing(chars: &[char], start: usize, open: &[char]) -> Option<usize> {
    let opener = start - open.len();
    let before_ok = opener == 0 || !chars[opener - 1].is_alphanumeric();
    let first = chars.get(start)?;
    if first.is_whitespace() || *first == open[0] || !before_ok {
        return None;
    }
    (start + 1..chars.len()).find(|&end| {
        chars[end..].starts_with(open)
            && !chars[end - 1].is_whitespace()
            && chars
                .get(end + open.len())
                .is_none_or(|after| !after.is_alphanumeric() && *after != open[0])
    })
}

fn link(chars: &[char], at: usize) -> Option<(String, String, usize)> {
    let label_end = (at + 1..chars.len()).find(|j| chars[*j] == ']')?;
    if chars[at + 1..label_end].contains(&'[') || chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let url_end = (label_end + 2..chars.len()).find(|j| chars[*j] == ')')?;
    let url = collect(chars, label_end + 2, url_end);
    let linkable = ["http://", "https://", "mailto:"].iter().any(|scheme| url.starts_with(scheme));
    if !linkable || url.chars().any(char::is_whitespace) {
        return None;
    }
    Some((collect(chars, at + 1, label_end), url, url_end))
}

fn address(url: &str) -> String {
    url.replace('&', "&amp;").replace('<', "%3C").replace('>', "%3E").replace('|', "%7C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_escapes_the_three_control_characters() {
        assert_eq!(mrkdwn("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
        assert_eq!(mrkdwn("<!channel> ping"), "&lt;!channel&gt; ping");
    }

    #[test]
    fn emphasis_is_rewritten_into_slack_marks() {
        assert_eq!(
            mrkdwn("**Done.** Shipped _today_, not *tomorrow*."),
            "*Done.* Shipped _today_, not _tomorrow_."
        );
        assert_eq!(mrkdwn("__bold__ and ~~gone~~"), "*bold* and ~gone~");
        assert_eq!(mrkdwn("**a _mixed_ run**"), "*a _mixed_ run*");
    }

    #[test]
    fn stray_marks_and_identifiers_are_left_as_written() {
        assert_eq!(mrkdwn("2 * 3 * 4 = 24"), "2 * 3 * 4 = 24");
        assert_eq!(mrkdwn("set max_tool_rounds and user_id"), "set max_tool_rounds and user_id");
        assert_eq!(mrkdwn("a lone ** here"), "a lone ** here");
    }

    #[test]
    fn code_keeps_its_text_and_still_escapes() {
        assert_eq!(mrkdwn("Run `cargo test **all**` now"), "Run `cargo test **all**` now");
        assert_eq!(mrkdwn("`Vec<u8>`"), "`Vec&lt;u8&gt;`");
        assert_eq!(
            mrkdwn("Here:\n```rust\nlet x = a && b;\n// **not bold**\n```\nDone."),
            "Here:\n```\nlet x = a &amp;&amp; b;\n// **not bold**\n```\nDone."
        );
        assert_eq!(mrkdwn("```\nleft open"), "```\nleft open\n```");
        assert_eq!(mrkdwn("```ls -la```"), "```ls -la```");
    }

    #[test]
    fn links_become_slack_links() {
        assert_eq!(
            mrkdwn(
                "See [the PR](https://github.com/o/r/pull/12) and [docs](https://x.dev/a?b=1&c=2)."
            ),
            "See <https://github.com/o/r/pull/12|the PR> and <https://x.dev/a?b=1&amp;c=2|docs>."
        );
        assert_eq!(mrkdwn("[a|b](https://x.dev/p>q)"), "<https://x.dev/p%3Eq|a|b>");
        assert_eq!(mrkdwn("[not a link] (here)"), "[not a link] (here)");
        assert_eq!(
            mrkdwn("[hi](!channel) [x](@U1) [y](!subteam^S1)"),
            "[hi](!channel) [x](@U1) [y](!subteam^S1)"
        );
        assert_eq!(mrkdwn("[mail](mailto:a@b.dev)"), "<mailto:a@b.dev|mail>");
    }

    #[test]
    fn headings_become_bold_lines_and_lists_are_kept() {
        assert_eq!(mrkdwn("## What **changed**"), "*What changed*");
        assert_eq!(mrkdwn("#hashtag"), "#hashtag");
        assert_eq!(
            mrkdwn("- one\n* two\n  + nested **x**\n1. first"),
            "\u{2022} one\n\u{2022} two\n  \u{2022} nested *x*\n1. first"
        );
        assert_eq!(mrkdwn("> quoted <tag>"), "> quoted &lt;tag&gt;");
        assert_eq!(mrkdwn("above\n---\nbelow"), "above\n---\nbelow");
    }

    #[test]
    fn a_real_agent_reply_reads_right_in_slack() {
        let reply = "## Summary\n\nI fixed the **retry backoff** in `llm/codex.rs`. The cap is now 3 & the delay doubles.\n\n### Next steps\n- Review [PR #12](https://github.com/acme/app/pull/12)\n- Run `./scripts/ci.sh rust`\n\n```sh\ncargo test -p guac --test slack\n```\n\nLet me know if 5 < 3 ever happens.";
        let want = "*Summary*\n\nI fixed the *retry backoff* in `llm/codex.rs`. The cap is now 3 &amp; the delay doubles.\n\n*Next steps*\n\u{2022} Review <https://github.com/acme/app/pull/12|PR #12>\n\u{2022} Run `./scripts/ci.sh rust`\n\n```\ncargo test -p guac --test slack\n```\n\nLet me know if 5 &lt; 3 ever happens.";
        assert_eq!(mrkdwn(reply), want);
    }

    #[test]
    fn a_question_with_options_keeps_its_shape() {
        let reply = "Which should I ship first?\n\n1. **Backoff fix** (small, _safe_)\n2. **Rewrite** of the queue\n\n~~Option 3~~ is off the table.";
        let want = "Which should I ship first?\n\n1. *Backoff fix* (small, _safe_)\n2. *Rewrite* of the queue\n\n~Option 3~ is off the table.";
        assert_eq!(mrkdwn(reply), want);
    }
}
