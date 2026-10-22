//! Binary blob writer for syncdown's FastSpannable.
//!
//! From comrak's `blob.rs` in syncdown (BSD-2-Clause, see `licenses/comrak.txt`): the writer
//! only, its comrak walk, tables and footnotes left behind.
//!
//! Produces a compact blob: `[text_len:4][span_count:3|flags:1][text][pad][spans][url_data]`
//! All positions are UTF-16 code units mapping directly to Java String indices.
//! Flag byte (byte 7): bit 0 = is_ascii, bit 1 = has_spoiler_body,
//! bits 2-7 reserved.
//!

use std::borrow::Cow;

use crate::url::{ResolvedUrl, extract_domain};
use crate::text::{prefer_ascii, typographic_symbols};

/// Span type constants, a copy of the file syncdown's Gradle `generateSpanTypes` task writes from
/// `library/span_types.toml`; kept in step with it by hand.
#[allow(missing_docs)]
mod span_types;

// Re-export at `crate::blob::*` so sibling modules (and `crate::tests::blob`)
// can reference span type ids without reaching into `span_types` directly.
pub(crate) use span_types::*;

const HEADER_SIZE: usize = 8;
const MAX_URL_LEN: usize = 4095;

/// Count UTF-16 code units from a UTF-8 string.
#[inline]
fn utf16_len(s: &str) -> usize {
    let mut len = 0;
    for &b in s.as_bytes() {
        if (b & 0b1100_0000) != 0b1000_0000 {
            len += 1;
        }
        if (b & 0b1111_1000) == 0b1111_0000 {
            len += 1;
        }
    }
    len
}

pub(crate) struct BlobWriter {
    blob: Vec<u8>,
    pub(crate) spans: Vec<i32>,
    pub(crate) url_data: Vec<u8>,
    len: usize,
    p: usize,
    /// Set if every byte written to the text section is ASCII (< 0x80).
    /// Stored as a header flag so consumers can skip their own scan.
    all_ascii: bool,
    /// When true, every `write_text` input is assumed ASCII (because the
    /// original markdown is ASCII), letting `write_text` skip the per-call
    /// `is_ascii` SIMD scan on long ASCII docs. `render_blob` verifies the
    /// assumption against the finished text; parser transforms that break
    /// it (smartypants, entities, emoji) trigger a slow-path re-render.
    fast_path_ascii: bool,
    /// Set if any LEMMY_SPOILER_CONTENT span exists. Stored as a header flag so
    /// consumers can skip hidden-spoiler containment checks for normal content.
    has_spoiler_body: bool,
    /// How many lists and quotes the walk is inside.
    list_depth: usize,
    quote_depth: usize,
}

impl BlobWriter {
    /// Construct a writer that runs the per-write `is_ascii` scan on every
    /// `write_text` call. Use when the input may contain non-ASCII bytes.
    pub(crate) fn new(cap: usize) -> Self {
        Self::with_fast_path(cap, false)
    }

    /// Construct a writer that skips the per-write `is_ascii` scan on the
    /// assumption that ASCII input yields ASCII output. `render_blob`
    /// verifies the finished text and re-renders with the scanning writer
    /// if a parser transform broke the assumption.
    pub(crate) fn new_ascii(cap: usize) -> Self {
        Self::with_fast_path(cap, true)
    }

    fn with_fast_path(cap: usize, fast_path_ascii: bool) -> Self {
        let mut blob = Vec::with_capacity(HEADER_SIZE + cap);
        blob.extend_from_slice(&[0u8; HEADER_SIZE]);
        // Pre-size spans/url_data to skip the doubling chain from 0. spans
        // holds 4 i32 per span; budget ~1 span per 40 input bytes (typical
        // density across the bench corpus). url_data is bytes; budget ~1
        // byte per 32 input bytes. Below ~96 chars the doc is too small to
        // make pre-allocation worthwhile (plain text has zero spans); leave
        // both at zero capacity, which std::vec::Vec specializes to no
        // allocation. Both clamp at the top to keep pathological input from
        // requesting megabytes up front.
        let (spans_cap, url_cap) = if cap < 96 {
            (0, 0)
        } else {
            ((cap / 40).min(2048) * 4, (cap / 32).min(8192))
        };
        Self {
            blob,
            spans: Vec::with_capacity(spans_cap),
            url_data: Vec::with_capacity(url_cap),
            len: 0,
            p: 0,
            all_ascii: true,
            fast_path_ascii,
            has_spoiler_body: false,
            list_depth: 0,
            quote_depth: 0,
        }
    }

    #[inline]
    pub(crate) fn pos(&self) -> usize {
        self.len + self.p
    }

    #[inline]
    pub(crate) fn write_text(&mut self, s: &str) {
        if self.p > 0 {
            self.blob.extend(std::iter::repeat_n(b'\n', self.p));
            self.len += self.p;
            self.p = 0;
        }
        self.blob.extend_from_slice(s.as_bytes());
        if self.fast_path_ascii {
            // Assumed ASCII: `all_ascii` stays true and byte length equals
            // UTF-16 length. `render_blob` checks the assumption against
            // the finished text; a violation discards this writer entirely.
            self.len += s.len();
            return;
        }
        let ascii = s.is_ascii();
        self.all_ascii &= ascii;
        self.len += if ascii { s.len() } else { utf16_len(s) };
    }

    pub(crate) fn nl(&mut self, n: usize) {
        self.p = self.p.max(n);
    }

    /// `write_text` for author prose (Text nodes): applies the typographic
    /// symbol substitution. A substitution always introduces non-ASCII; when
    /// everything written so far is genuinely ASCII the accumulated UTF-16
    /// length is still exact, so the writer can drop the fast path in place
    /// and skip the re-render. Decoded entities and shortcode emoji break
    /// that premise — they write non-ASCII under fast byte-as-unit accounting
    /// — so with a dirty prefix the flag stays set and the final scan in
    /// `render_blob` re-renders with exact accounting, as it did before
    /// symbols moved here.
    fn write_prose(&mut self, s: &str) {
        // One gate for both character passes: prefer_ascii only acts on
        // 0xE2-lead sequences, typographic_symbols only on '(' or '+'.
        // The common clean node takes a single scan and writes as-is.
        if memchr::memchr3(0xE2, b'(', b'+', s.as_bytes()).is_none() {
            return self.write_text(s);
        }
        let s = prefer_ascii(s);
        match typographic_symbols(&s) {
            Cow::Borrowed(_) => self.write_text(&s),
            Cow::Owned(sym) => {
                if !self.fast_path_ascii || self.text().is_ascii() {
                    self.fast_path_ascii = false;
                }
                self.write_text(&sym);
            }
        }
    }

    /// True if the next byte written would land at the start of a line.
    /// Either nothing has been written, the last byte is a newline, or
    /// pending newlines are queued for flush.
    pub(crate) fn at_line_start(&self) -> bool {
        self.p > 0 || self.len == 0 || self.blob.last() == Some(&b'\n')
    }

    /// Drop pending (unflushed) newlines. Test-only.
    #[cfg(test)]
    pub(crate) fn clear_pending(&mut self) {
        self.p = 0;
    }

    /// Rendered text bytes so far (excluding header). Used during render and by tests.
    pub(crate) fn text(&self) -> &str {
        // Only `&str` slices and newlines are ever written past the header, so the bytes are UTF-8 by construction
        unsafe { std::str::from_utf8_unchecked(&self.blob[HEADER_SIZE..]) }
    }

    pub(crate) fn span(&mut self, t: i32, start: usize) {
        self.span_data(t, start, 0);
    }

    pub(crate) fn span_data(&mut self, t: i32, start: usize, data: i32) {
        if start < self.len {
            self.spans
                .extend_from_slice(&[start as i32, self.len as i32, t, data]);
            self.has_spoiler_body |= t == LEMMY_SPOILER_CONTENT;
        }
    }

    pub(crate) fn span_url(&mut self, t: i32, start: usize, url: &ResolvedUrl) {
        if start >= self.len {
            return;
        }
        // Takes only a ResolvedUrl, so no caller can emit a raw URL and none is resolved twice.
        let offset = self.url_data.len();
        // Back a capped cut off to a char boundary; the host decodes this slice as UTF-8.
        let mut url_len = url.len().min(MAX_URL_LEN);
        while !url.is_char_boundary(url_len) {
            url_len -= 1;
        }
        self.url_data.extend_from_slice(&url.as_bytes()[..url_len]);
        self.spans.extend_from_slice(&[
            start as i32,
            self.len as i32,
            t,
            ((offset as i32) << 12) | (url_len as i32),
        ]);
    }

    fn emit_image(&mut self, url: &ResolvedUrl) {
        if self.len != 0 {
            // Separate the image from prior content by at least two newlines;
            // existing trailing `\n` (past any trailing spaces/tabs) count.
            let existing = self.blob[HEADER_SIZE..]
                .iter()
                .rev()
                .skip_while(|&&b| matches!(b, b' ' | b'\t'))
                .take_while(|&&b| b == b'\n')
                .count();
            self.nl(2_usize.saturating_sub(existing));
        }
        let start = self.pos();
        self.write_text("\u{0001}");
        self.span_url(IMAGE, start, url);
        self.nl(2);
    }

    fn append_domain_suffix(&mut self, text_start: usize, url: &str) {
        let Some(domain) = extract_domain(url) else {
            return;
        };
        let needle = domain.as_bytes();
        if self.blob[text_start..]
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
        {
            return;
        }
        self.write_text(" (");
        self.write_text(&domain);
        self.write_text(")");
    }

    pub(crate) fn into_blob(mut self) -> Vec<u8> {
        let txt_len = self.blob.len() - HEADER_SIZE;
        let span_count = self.spans.len() / 4;

        let spans_view = unsafe {
            let ptr = self.spans.as_mut_ptr() as *mut [i32; 4];
            std::slice::from_raw_parts_mut(ptr, span_count)
        };
        // Pack the sort key (start asc, end desc, type asc) into a single u64
        // so the comparator is a plain u64 compare instead of a 3-field tuple
        // walk. Layout: bits 63..32 = start, bits 31..8 = inverted 24-bit end
        // (for descending order), bits 7..0 = span type. Safe because start/end
        // are non-negative text offsets (well under 2^24 for all realistic
        // documents) and span types fit in u8.
        let key = |s: &[i32; 4]| {
            let start = s[0] as u32 as u64;
            let end_inv = (0xFF_FFFF - (s[1] as u32 & 0xFF_FFFF)) as u64;
            let ty = (s[2] as u32 & 0xFF) as u64;
            (start << 32) | (end_inv << 8) | ty
        };
        // Spans arrive after their node's children, so each only has to move back past its own
        // descendants: an insertion sort is linear plus those moves. Deep nesting could make the
        // moves quadratic, so past a budget per span the rest goes to the general sort.
        let mut budget = span_count * 8;
        let mut i = 1;
        while i < span_count && budget > 0 {
            let cur = spans_view[i];
            let k = key(&cur);
            let mut j = i;
            while j > 0 && key(&spans_view[j - 1]) > k {
                spans_view[j] = spans_view[j - 1];
                j -= 1;
            }
            budget = budget.saturating_sub(i - j);
            spans_view[j] = cur;
            i += 1;
        }
        if i < span_count {
            spans_view.sort_unstable_by_key(key);
        }

        // Header layout: text_len in bytes 0-3, span_count in bytes 4-6, flags
        // in byte 7. span_count is capped at 24 bits (16M); the high byte is
        // reused as a flag field so the header stays 8 bytes.
        //   bit 0: is_ascii         — text section is pure ASCII
        //   bit 1: has_spoiler_body — has a LEMMY_SPOILER_CONTENT span
        //   bits 2-7: reserved
        let flags: u32 = (self.all_ascii as u32) | ((self.has_spoiler_body as u32) << 1);
        let count_with_flags = (span_count as u32 & 0x00FF_FFFF) | (flags << 24);
        self.blob[0..4].copy_from_slice(&(txt_len as i32).to_le_bytes());
        self.blob[4..8].copy_from_slice(&count_with_flags.to_le_bytes());

        let padding = (4 - (txt_len % 4)) % 4;
        self.blob.extend_from_slice(&[0u8; 3][..padding]);

        let span_bytes = unsafe {
            std::slice::from_raw_parts(self.spans.as_ptr() as *const u8, self.spans.len() * 4)
        };
        self.blob.extend_from_slice(span_bytes);
        self.blob.extend_from_slice(&self.url_data);
        self.blob
    }
}
