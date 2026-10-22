//! One pass over a comment's HTML, writing its words and spans as it goes: no tree, and no
//! markdown between. Each tag is written as comrak's blob writer writes the markdown it stands
//! for, so a comment reads the same from either.

use super::*;
use crate::entities::{decode_html_attribute_entities, decode_html_entities_into};
use crate::image_url::is_image_url;
use crate::scan::{is_whitespace, process_comment_or_doctype, scan_tag};
use crate::url::resolve_url;

/// A tag open around words still to come.
enum Open<'a> {
    /// Bold or italic: the span it closes with and the tag that closes it.
    Style(i32, &'static str, usize),
    /// A link: where its span and its words' bytes start, and its address, borrowed from the
    /// input unless an entity was decoded in it.
    Link(usize, usize, Cow<'a, str>),
}

struct Walk<'a, 'o> {
    html: &'a str,
    out: &'o mut BlobWriter,
    open: Vec<Open<'a>>,
    /// The words between two tags with their entities decoded, one buffer for the whole walk.
    decoded: String,
    /// Nothing of the current paragraph is written yet.
    block_start: bool,
    /// The `>` marks read at the paragraph's start, before its first word.
    marks: usize,
    /// Where the paragraph's quote starts and how many levels it opened.
    quote: Option<(usize, usize)>,
    /// Whitespace read since the last word, written as one space before the next.
    space: bool,
    /// A `<pre>`'s words so far, written whole at its end.
    pre: Option<String>,
}

pub(crate) fn visit(html: &str, out: &mut BlobWriter) {
    let mut w = Walk {
        html,
        out,
        open: Vec::new(),
        decoded: String::new(),
        block_start: true,
        marks: 0,
        quote: None,
        space: false,
        pre: None,
    };
    let bytes = html.as_bytes();
    let (mut i, mut from) = (0, 0);
    while let Some(at) = memchr::memchr(b'<', &bytes[from..]).map(|at| from + at) {
        let next = bytes.get(at + 1).copied().unwrap_or(0);
        // A `<` that opens no tag is a word
        if !(next.is_ascii_alphabetic() || next == b'!' || next == b'/' && bytes.get(at + 2).is_some_and(u8::is_ascii_alphabetic)) {
            from = at + 1;
            continue;
        }
        w.text(&html[i..at]);
        i = w.tag(at);
        from = i;
    }
    w.text(&html[i..]);
    w.end();
}

/// HTML's whitespace: what a run of it between words collapses from.
fn is_space(c: char) -> bool {
    c.is_ascii() && is_whitespace(c as u8)
}

/// A run of words with its whitespace one space each, as comrak's `collapse_whitespace` makes a
/// text node's; borrowed when it already is, which one pass decides.
fn collapse(s: &str) -> Cow<'_, str> {
    let bytes = s.as_bytes();
    let clean = !bytes
        .iter()
        .enumerate()
        .any(|(i, &c)| is_whitespace(c) && (c != b' ' || bytes.get(i + 1) == Some(&b' ')));
    if clean {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !is_space(c) {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    Cow::Owned(out)
}

impl<'a> Walk<'a, '_> {
    /// Reads the tag opening at `at`, and answers where the words after it begin. A tag the input
    /// ends inside is no text, as HTML drops it.
    fn tag(&mut self, at: usize) -> usize {
        let (html, bytes) = (self.html, self.html.as_bytes());
        if bytes[at + 1] == b'!' {
            let comment = process_comment_or_doctype(html, at);
            return if comment.complete { comment.new_position } else { html.len() };
        }
        let closing = bytes[at + 1] == b'/';
        let name_start = at + 1 + closing as usize;
        let name_end = bytes[name_start..]
            .iter()
            .position(|&c| !c.is_ascii_alphanumeric())
            .map_or(bytes.len(), |n| name_start + n);
        let name = &html[name_start..name_end];
        if closing {
            let Some(end) = memchr::memchr(b'>', &bytes[name_end..]) else {
                return html.len();
            };
            self.close(name);
            return name_end + end + 1;
        }
        let mut href = None;
        let scan = scan_tag(html, name_end, &["href"], |_, value| href = Some(value));
        if !scan.complete {
            return html.len();
        }
        self.start(name, href, scan.new_position)
    }

    /// Opens a tag, answering where the words after it begin: past the link for a picture.
    fn start(&mut self, name: &str, href: Option<&'a str>, after: usize) -> usize {
        let is = |tag: &str| name.eq_ignore_ascii_case(tag);
        if let Some(pre) = &mut self.pre {
            if is("br") {
                pre.push('\n');
            }
            return after;
        }
        if is("p") {
            self.end_block();
        } else if is("br") {
            self.out.nl(1);
            self.space = false;
        } else if is("pre") {
            self.end_block();
            self.pre = Some(String::new());
        } else if is("b") || is("strong") || is("i") || is("em") {
            self.begin();
            let (span, closer) = match name.len() {
                1 if is("b") => (BOLD, "b"),
                6 => (BOLD, "strong"),
                1 => (ITALIC, "i"),
                _ => (ITALIC, "em"),
            };
            self.open.push(Open::Style(span, closer, self.out.pos()));
        } else if is("a")
            && let Some(href) = href
            && !self.open.iter().any(|o| matches!(o, Open::Link(..)))
        {
            self.begin();
            let href = decode_html_attribute_entities(href);
            if let Some(past) = self.picture(&href, after) {
                return past;
            }
            self.open.push(Open::Link(self.out.pos(), self.out.blob.len(), href));
        }
        after
    }

    /// A link whose only words are an address, to a picture, is the picture, as comrak's autolink
    /// is. Answers where the words after its `</a>` begin.
    fn picture(&mut self, href: &str, after: usize) -> Option<usize> {
        let bytes = self.html.as_bytes();
        let lt = after + memchr::memchr(b'<', &bytes[after..])?;
        let close = bytes.get(lt..lt + 3)?;
        if !close.eq_ignore_ascii_case(b"</a") {
            return None;
        }
        let raw = &self.html[after..lt];
        let words = if decode_html_entities_into(raw, false, false, &mut self.decoded) { &self.decoded } else { raw };
        if !(words.starts_with("http://") || words.starts_with("https://")) {
            return None;
        }
        let url = resolve_url(href);
        if !is_image_url(&url) {
            return None;
        }
        let gt = lt + memchr::memchr(b'>', &bytes[lt..])?;
        self.out.emit_image(&url);
        Some(gt + 1)
    }

    fn close(&mut self, name: &str) {
        let is = |tag: &str| name.eq_ignore_ascii_case(tag);
        if self.pre.is_some() {
            if is("pre") {
                self.end_pre();
            }
            return;
        }
        if is("p") {
            self.end_block();
        } else if is("a") {
            if let Some(at) = self.open.iter().rposition(|o| matches!(o, Open::Link(..))) {
                let link = self.open.remove(at);
                self.end_open(link);
            }
        } else if let Some(at) = self
            .open
            .iter()
            .rposition(|o| matches!(o, Open::Style(_, closer, _) if is(closer)))
        {
            let style = self.open.remove(at);
            self.end_open(style);
        }
    }

    fn end_open(&mut self, open: Open<'a>) {
        match open {
            Open::Style(span, _, start) => self.out.span(span, start),
            Open::Link(start, text_start, href) => {
                let url = resolve_url(&href);
                self.out.span_url(LINK, start, &url);
                self.out.span(LINK_SIZE, start);
                self.out.append_domain_suffix(text_start, &url);
            }
        }
    }

    /// The words between two tags, their entities decoded and their whitespace one space.
    fn text(&mut self, raw: &str) {
        if raw.is_empty() {
            return;
        }
        if memchr::memchr(b'&', raw.as_bytes()).is_none() {
            return self.words(raw);
        }
        // Taken out while the words are written, so they can borrow it
        let mut decoded = std::mem::take(&mut self.decoded);
        if decode_html_entities_into(raw, false, false, &mut decoded) {
            self.words(&decoded);
        } else {
            self.words(raw);
        }
        self.decoded = decoded;
    }

    /// Words with their entities decoded, written with their whitespace one space.
    fn words(&mut self, text: &str) {
        if let Some(pre) = &mut self.pre {
            // HTML drops the newline straight after `<pre>`
            let text = if pre.is_empty() { text.strip_prefix('\n').unwrap_or(&text) } else { &text };
            pre.push_str(text);
            return;
        }
        let mut run: &str = &text;
        if self.block_start {
            // A paragraph opening on `>` is a quote, the marks no text
            loop {
                run = run.trim_start_matches(is_space);
                let rest = run.trim_start_matches('>');
                if rest.len() == run.len() {
                    break;
                }
                self.marks += run.len() - rest.len();
                run = rest;
            }
        }
        let words = run.trim_matches(is_space);
        if words.len() < run.len() && run.starts_with(is_space) {
            self.space = true;
        }
        if !words.is_empty() {
            self.begin();
            self.out.write_prose(&collapse(words));
            self.space = run.ends_with(is_space);
        }
    }

    /// Before anything of a paragraph is written: opens its quote, or writes the space before it.
    fn begin(&mut self) {
        if self.block_start {
            self.block_start = false;
            if self.marks > 0 {
                self.out.quote_depth += self.marks;
                self.quote = Some((self.out.pos(), self.marks));
            }
        } else if self.space {
            self.out.write_text(" ");
        }
        self.space = false;
    }

    /// Ends the paragraph: closes its quote, and leaves a blank line before whatever follows.
    fn end_block(&mut self) {
        if let Some((start, levels)) = self.quote.take() {
            // Innermost first, as comrak closes them
            for _ in 0..levels {
                self.out.quote_depth -= 1;
                self.out.span_data(QUOTE, start, self.out.quote_depth as i32);
            }
        }
        if self.out.pos() > 0 {
            self.out.nl(2);
        }
        self.block_start = true;
        self.marks = 0;
        self.space = false;
    }

    fn end_pre(&mut self) {
        let pre = self.pre.take().unwrap_or_default();
        let code = pre.trim_end();
        if !code.is_empty() {
            let start = self.out.pos();
            self.out.write_text(code);
            self.out.span(CODE_BLOCK, start);
        }
        self.end_block();
    }

    /// Closes whatever the input left open, as HTML does at its end.
    fn end(mut self) {
        if self.pre.is_some() {
            self.end_pre();
        }
        while let Some(open) = self.open.pop() {
            self.end_open(open);
        }
        self.end_block();
    }
}
