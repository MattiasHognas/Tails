//! Boundary-safe text helpers.
//!
//! Datadog text routinely contains multibyte characters (Swedish `åäö`, emoji,
//! CJK, combining marks). Slicing or truncating a `String` at a byte offset that
//! is not a char boundary panics, so every byte-limited cut goes through
//! [`truncate_bytes`] instead of `String::truncate` or `&s[..n]`.

use std::borrow::Cow;

/// Appended by [`truncate_with_marker`] callers that shorten prompt excerpts.
pub const TRUNCATION_MARKER: &str = " …[truncated]";

/// Characters that attach to the preceding character: combining marks,
/// variation selectors, zero-width joiner and emoji skin-tone modifiers.
/// Cutting right before one of them would separate it from its base.
pub(crate) fn is_extender(c: char) -> bool {
    matches!(c,
        '\u{0300}'..='\u{036F}'     // combining diacritical marks
        | '\u{1AB0}'..='\u{1AFF}'   // combining diacritical marks extended
        | '\u{1DC0}'..='\u{1DFF}'   // combining diacritical marks supplement
        | '\u{20D0}'..='\u{20FF}'   // combining marks for symbols
        | '\u{FE20}'..='\u{FE2F}'   // combining half marks
        | '\u{FE00}'..='\u{FE0F}'   // variation selectors
        | '\u{200D}'                // zero-width joiner
        | '\u{1F3FB}'..='\u{1F3FF}' // emoji skin-tone modifiers
        | '\u{E0100}'..='\u{E01EF}' // variation selectors supplement
    )
}

/// The longest prefix of `s` that is at most `max_bytes` long, ends on a char
/// boundary and does not split a character from a following combining mark,
/// variation selector, skin-tone modifier or zero-width joiner sequence.
///
/// Never panics; returns `s` unchanged when it already fits.
pub fn truncate_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut cut = max_bytes;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    // Back off while the first dropped char attaches to the last kept one, or the
    // last kept char is a joiner (which would dangle without its successor).
    loop {
        let next = s[cut..].chars().next();
        let prev = s[..cut].chars().next_back();
        match (prev, next) {
            (Some(p), Some(n)) if is_extender(n) || p == '\u{200D}' => {
                cut -= p.len_utf8();
            }
            _ => break,
        }
    }
    &s[..cut]
}

/// [`truncate_bytes`] to `max_bytes`, appending `marker` when anything was cut.
/// The marker is not counted towards `max_bytes`.
pub fn truncate_with_marker<'a>(s: &'a str, max_bytes: usize, marker: &str) -> Cow<'a, str> {
    let head = truncate_bytes(s, max_bytes);
    if head.len() == s.len() {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(format!("{head}{marker}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples mixing 1-, 2-, 3- and 4-byte chars, combining marks and ZWJ emoji.
    const SAMPLES: &[&str] = &[
        "Återförsök misslyckades för användare åsa.öberg ",
        "決済ゲートウェイ タイムアウト ",
        "deploy 🚀 failed 🔥🔥 ",
        "e\u{0301}te\u{0301} cafe\u{0301} ",
        "on-call 👩\u{200D}💻 paged 👍\u{1F3FD} ",
        "plain ascii ",
    ];

    #[test]
    fn short_text_is_returned_unchanged() {
        assert_eq!(truncate_bytes("åäö", 6), "åäö");
        assert_eq!(truncate_bytes("", 0), "");
        assert!(matches!(
            truncate_with_marker("åäö", 6, TRUNCATION_MARKER),
            Cow::Borrowed("åäö")
        ));
    }

    #[test]
    fn cuts_at_the_last_char_boundary_not_after_the_limit() {
        // "å" is 2 bytes: a limit of 3 falls inside the second "å".
        assert_eq!(truncate_bytes("ååå", 3), "å");
        assert_eq!(truncate_bytes("ååå", 4), "åå");
        // A 4-byte emoji is dropped entirely when the limit falls inside it.
        assert_eq!(truncate_bytes("a🔥b", 2), "a");
        assert_eq!(truncate_bytes("🔥", 3), "");
        assert_eq!(truncate_bytes("決済", 5), "決");
    }

    #[test]
    fn combining_marks_and_joiners_stay_with_their_base() {
        // "é" as e + U+0301: never keep the bare "e" without its accent.
        assert_eq!(truncate_bytes("ae\u{0301}", 2), "a");
        assert_eq!(truncate_bytes("ae\u{0301}x", 4), "ae\u{0301}");
        // Woman technologist: 👩 ZWJ 💻 is kept whole or dropped whole.
        let s = "x👩\u{200D}💻";
        for max in 1..s.len() {
            assert_eq!(truncate_bytes(s, max), "x", "max {max}");
        }
        assert_eq!(truncate_bytes(s, s.len()), s);
        // Skin-tone modifier stays with its emoji.
        assert_eq!(truncate_bytes("👍\u{1F3FD}", 5), "");
    }

    /// Property-style sweep: every limit on every sample (repeated past a
    /// realistic excerpt size) yields a valid, bounded prefix of the input.
    #[test]
    fn every_limit_yields_a_bounded_valid_prefix() {
        for sample in SAMPLES {
            let text = sample.repeat(1600 / sample.len() + 2);
            for max in (0..=text.len() + 2).filter(|m| *m < 64 || (1490..=1510).contains(m)) {
                let head = truncate_bytes(&text, max);
                assert!(head.len() <= max, "{sample:?} max {max}");
                assert!(text.starts_with(head));
                // At most one grapheme-ish unit (≤ 4 chars of up to 4 bytes) is lost.
                assert!(
                    max.min(text.len()) - head.len() <= 16,
                    "{sample:?} max {max}"
                );
                let marked = truncate_with_marker(&text, max, TRUNCATION_MARKER);
                assert_eq!(marked.ends_with(TRUNCATION_MARKER), head.len() < text.len());
            }
        }
    }
}
