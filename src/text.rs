//! From comrak's `text.rs` in syncdown (BSD-2-Clause, see `licenses/comrak.txt`).
//!
//! Text normalization for the blob renderer — the whole policy, in one place.
//!
//! Author prose renders as typed, with exactly three exceptions:
//! - runs of spaces collapse to one ([`collapse_whitespace`])
//! - typed typographic Unicode (curly quotes, en/em dashes, ellipsis)
//!   normalizes to ASCII, keeping the zero-copy path wide ([`prefer_ascii`])
//! - the four symbol codes `(c)` `(r)` `(tm)` `+-` render as ©®™±
//!   ([`typographic_symbols`])
//!
//! The last two point in opposite directions but act on disjoint character
//! sets, so they cannot fight: the passes commute and the pipeline is
//! idempotent (pinned by `tests::smart_parity`). `parse.smart` is
//! deliberately off in production — its parser-side transforms either
//! duplicated this policy at 4x the AST cost or contradicted it; see the
//! smart_parity suite for the retirement contract.

use std::borrow::Cow;

/// Collapse runs of two or more spaces into a single space. Zero-copy fast
/// path when the input has no double-space (the common case).
#[inline]
pub fn collapse_whitespace(s: &str) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    // Small inputs (most inline Text runs after tokenization): a simple
    // windows(2) byte loop already auto-vectorizes to a tight check, and
    // beats memmem's per-call SIMD setup cost. Larger inputs (long paragraph
    // fragments) flip the trade-off — memmem's SIMD search dominates.
    let has_double = if bytes.len() < 64 {
        bytes.windows(2).any(|w| w[0] == b' ' && w[1] == b' ')
    } else {
        // Built once: memmem::find would construct a searcher on every call
        static DOUBLE_SPACE: std::sync::OnceLock<memchr::memmem::Finder<'static>> =
            std::sync::OnceLock::new();
        DOUBLE_SPACE
            .get_or_init(|| memchr::memmem::Finder::new(b"  "))
            .find(bytes)
            .is_some()
    };
    if !has_double {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    Cow::Owned(out)
}

/// Substitute common typographic chars with ASCII equivalents:
/// - U+2018/U+2019 (curly single quotes) → `'`
/// - U+201C/U+201D (curly double quotes) → `"`
/// - U+2013 (en dash) / U+2014 (em dash) → `-`
/// - U+2026 (horizontal ellipsis) → `...`
///
/// All six share the `E2 80 X` UTF-8 prefix; one window scan covers them.
/// Zero-copy fast path when none are present (the common case).
#[inline]
pub fn prefer_ascii(s: &str) -> Cow<'_, str> {
    // Fast path: all typographic chars start with `0xE2` in UTF-8. memchr
    // SIMD-scans for it in one shot; ASCII-only text returns immediately.
    if memchr::memchr(0xE2, s.as_bytes()).is_none() {
        return Cow::Borrowed(s);
    }
    if !s.as_bytes().windows(3).any(|w| {
        w[0] == 0xE2
            && w[1] == 0x80
            && matches!(w[2], 0x93 | 0x94 | 0x98 | 0x99 | 0x9C | 0x9D | 0xA6)
    }) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\u{2018}' | '\u{2019}' => out.push('\''),
            '\u{201C}' | '\u{201D}' => out.push('"'),
            '\u{2013}' | '\u{2014}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            _ => out.push(ch),
        }
    }
    Cow::Owned(out)
}

/// Substitute the typographic symbols production keeps after retiring
/// `parse.smart`: `(c)`/`(C)` → ©, `(r)`/`(R)` → ®, `(tm)`/`(TM)` → ™,
/// `+-` → ±. Matches the retired parser handler exactly: the all-lower and
/// all-upper pairs only (`(Tm)` stays literal), anywhere in the text, no
/// word-boundary requirement. Zero-copy when nothing matches (the common
/// case); the memchr iterator SIMD-skips between candidate bytes.
///
/// A `Cow::Owned` return always contains non-ASCII — callers on the blob's
/// ASCII fast path use that to downgrade the writer instead of re-rendering.
#[inline]
pub fn typographic_symbols(s: &str) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0;
    for i in memchr::memchr2_iter(b'(', b'+', bytes) {
        if i < copied {
            continue; // candidate byte inside an already-replaced region
        }
        let (rep, len) = match &bytes[i..] {
            [b'(', b'c' | b'C', b')', ..] => ("\u{a9}", 3),
            [b'(', b'r' | b'R', b')', ..] => ("\u{ae}", 3),
            [b'(', b't', b'm', b')', ..] | [b'(', b'T', b'M', b')', ..] => ("\u{2122}", 4),
            [b'+', b'-', ..] => ("\u{b1}", 2),
            _ => continue,
        };
        let out = out.get_or_insert_with(|| String::with_capacity(s.len()));
        out.push_str(&s[copied..i]);
        out.push_str(rep);
        copied = i + len;
    }
    match out {
        Some(mut out) => {
            out.push_str(&s[copied..]);
            Cow::Owned(out)
        }
        None => Cow::Borrowed(s),
    }
}
