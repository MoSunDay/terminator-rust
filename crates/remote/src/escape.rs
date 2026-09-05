//! POSIX shell quoting helpers.
//!
//! All remote-side command strings are assembled here so that every
//! interpolated fragment is either single-quoted or delivered through a
//! quoted (non-expanding) heredoc. Nothing in this module executes a
//! shell; these are pure string builders.

/// Characters that make a string unsafe to interpolate verbatim into a
/// shell command line.
const SHELL_METACHARS: &str = "`$&;|<>()\\\"'*?[]#~=%!\t\n\r ";

/// True when `s` contains any character with special meaning to POSIX sh.
///
/// Used defensively: callers route any such string through
/// [`sh_single_quote`] or [`heredoc_literal`] instead of splicing it raw.
pub fn contains_shell_metachar(s: &str) -> bool {
    s.chars().any(|c| SHELL_METACHARS.contains(c))
}

/// Wrap `s` in POSIX single quotes, escaping embedded single quotes as
/// `'\''` (close quote, escaped quote, reopen quote).
///
/// The result is a single shell word that expands to exactly `s`: no
/// parameter, command, or pathname expansion happens inside single quotes.
pub fn sh_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Quote each part with [`sh_single_quote`] and join with single spaces.
///
/// Empty input yields the empty string.
pub fn sh_quote_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| sh_single_quote(p))
        .collect::<Vec<String>>()
        .join(" ")
}

/// Build a quoted (literal) heredoc: `<<'TAG'\n<body>\nTAG`.
///
/// Because the tag is quoted, the body is taken verbatim: `$var`,
/// backticks and friends are NOT expanded. The body is guaranteed to end
/// with a newline before the terminator (POSIX heredocs are line based,
/// so a trailing newline is the only faithfully representable form).
///
/// The caller chooses the tag; the body must not contain a line equal to
/// the tag or the heredoc would terminate early. This is asserted in
/// debug builds and covered by tests for all bodies we ship.
pub fn heredoc_literal(tag: &str, body: &str) -> String {
    debug_assert!(
        !body.lines().any(|line| line == tag),
        "heredoc body contains terminator line {tag:?}"
    );
    let mut out = String::with_capacity(body.len() + tag.len() * 2 + 8);
    out.push_str("<<'");
    out.push_str(tag);
    out.push_str("'\n");
    out.push_str(body);
    if !body.is_empty() && !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(tag);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Model POSIX single-quote word parsing: given the output of
    /// [`sh_single_quote`], recover the original string.
    fn posix_unquote_single(quoted: &str) -> String {
        let chars: Vec<char> = quoted.chars().collect();
        assert_eq!(chars.first(), Some(&'\''), "must open with a quote");
        let mut out = String::new();
        let mut i = 1;
        let mut in_quote = true;
        while i < chars.len() {
            let c = chars[i];
            if in_quote {
                if c == '\'' {
                    in_quote = false;
                } else {
                    out.push(c);
                }
            } else {
                // Outside quotes our producer only ever emits the escape
                // sequence `\'` followed by a quote that reopens the string
                // (`'\''` = close, escaped quote, reopen).
                assert_eq!(c, '\\', "unexpected char outside quotes");
                assert_eq!(chars.get(i + 1), Some(&'\''), "expected escaped quote");
                out.push('\'');
                assert_eq!(chars.get(i + 2), Some(&'\''), "expected reopen quote");
                in_quote = true;
                i += 2; // bottom +=1 lands just past the reopen quote
            }
            i += 1;
        }
        assert!(!in_quote, "must close the final quote");
        out
    }

    #[test]
    fn single_quote_roundtrips_adversarial_strings() {
        let cases = [
            "",
            "plain",
            "it's here",
            "'leading and trailing'''",
            "$(rm -rf /)",
            "`id`",
            "semi;colon|pipe&bg",
            "line one\nline two",
            "tab\there",
            "a\\b\"c'd\"e",
            "space  separated",
            "caf\0e-no-nul-but-unicode: \u{4e2d}\u{6587}",
            "~~~~~~~~",
            "#comment ~ % != ==",
        ];
        for case in cases {
            let quoted = sh_single_quote(case);
            assert!(
                quoted.starts_with('\'') && quoted.ends_with('\''),
                "{quoted:?}"
            );
            assert_eq!(posix_unquote_single(&quoted), case);
        }
    }

    #[test]
    fn single_quote_escapes_embedded_quotes() {
        assert_eq!(sh_single_quote("a'b"), "'a'\\''b'");
        // A lone quote: empty string + escaped quote + empty string.
        assert_eq!(sh_single_quote("'"), "''\\'''");
        assert_eq!(sh_single_quote("''"), "''\\'''\\'''");
    }

    #[test]
    fn quote_join_quotes_and_separates() {
        let parts = vec![
            "echo".to_string(),
            "two words".to_string(),
            "$HOME".to_string(),
        ];
        assert_eq!(sh_quote_join(&parts), "'echo' 'two words' '$HOME'");
        assert_eq!(sh_quote_join(&[]), "");
    }

    #[test]
    fn heredoc_literal_is_quoted_and_verbatim() {
        let body = "price=$5 `whoami` $(date)\nsecond line\n";
        let hd = heredoc_literal("ZT_EOF", body);
        assert!(
            hd.starts_with("<<'ZT_EOF'\n"),
            "quoted tag form required: {hd:?}"
        );
        assert!(hd.ends_with("\nZT_EOF"));
        // The whole body appears verbatim between tag lines: no expansion
        // markers are added or removed by the helper.
        assert!(hd.contains(body.trim_end()));
        assert_eq!(hd, "<<'ZT_EOF'\n".to_string() + body + "ZT_EOF");
    }

    #[test]
    fn heredoc_appends_missing_trailing_newline() {
        assert_eq!(heredoc_literal("EOF", "x"), "<<'EOF'\nx\nEOF");
        // Empty body is an empty heredoc: no blank content line.
        assert_eq!(heredoc_literal("EOF", ""), "<<'EOF'\nEOF");
    }

    #[test]
    fn metachar_detection_covers_all_specials() {
        assert!(!contains_shell_metachar("plain-Word_1.2/3"));
        for c in [
            '`', '$', '&', ';', '|', '<', '>', '(', ')', '\\', '"', '\'', '*', '?', '[', ']', '#',
            '~', '=', '%', '!', ' ', '\t', '\n',
        ] {
            let s = format!("a{c}b");
            assert!(contains_shell_metachar(&s), "metachar {c:?} not detected");
        }
    }
}
