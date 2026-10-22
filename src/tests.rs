//! Tests for [`crate::parse`], Hacker News' comment HTML as syncdown's blob.
//!
//! Every expectation is comrak's blob for the same comment written as markdown, read off syncdown's
//! comrak on 22 Oct 2026, so the two writers show a comment the same way.
//!
//! - `nothing`    : the null contract, shared with syncdown's `parse`
//! - `paragraphs` : a bare `<p>` opens every paragraph but the first; whitespace collapses
//! - `entities`   : decoded after the tags are read, in UTF-16 positions
//! - `quotes`     : a paragraph opening on `>` is a quote, as the site's posters write one
//! - `links`      : the address resolved, the domain added where the words lack it, pictures
//! - `emphasis`   : italic and bold
//! - `code`       : preformatted code kept as written
//! - `words`      : markdown's characters, tags no site rule reads, comments, typography
//! - `whole`      : a real comment

use crate::parse;
use crate::blob::{BOLD, CODE_BLOCK, IMAGE, ITALIC, LINK, LINK_SIZE, QUOTE};

// ── helpers ──────────────────────────────────────────────────────────────

/// A span as the blob holds it; a link's or picture's data is its address.
#[derive(Debug, PartialEq)]
enum Data {
    Int(i32),
    Url(String),
}

#[derive(Debug, PartialEq)]
struct Span(i32, i32, i32, Data);

fn span(start: i32, end: i32, t: i32) -> Span {
    Span(start, end, t, Data::Int(0))
}

fn quote(start: i32, end: i32, depth: i32) -> Span {
    Span(start, end, QUOTE, Data::Int(depth))
}

fn url(start: i32, end: i32, t: i32, url: &str) -> Span {
    Span(start, end, t, Data::Url(url.to_string()))
}

/// A link's two spans, as comrak writes them.
fn link(start: i32, end: i32, address: &str) -> [Span; 2] {
    [url(start, end, LINK, address), span(start, end, LINK_SIZE)]
}

struct Blob {
    text: String,
    ascii: bool,
    spans: Vec<Span>,
}

/// Reads a blob back: the header, the text, the spans past the padding and their addresses.
fn read(blob: &[u8]) -> Blob {
    let int = |at: usize| i32::from_le_bytes(blob[at..at + 4].try_into().unwrap());
    let text_len = int(0) as usize;
    let header = int(4) as u32;
    let count = (header & 0xFF_FFFF) as usize;
    let text = std::str::from_utf8(&blob[8..8 + text_len]).unwrap().to_string();
    let spans_at = 8 + text_len + (4 - text_len % 4) % 4;
    let urls = &blob[spans_at + count * 16..];
    let spans = (0..count)
        .map(|i| {
            let at = spans_at + i * 16;
            let (start, end, t, data) = (int(at), int(at + 4), int(at + 8), int(at + 12));
            let data = if t == LINK || t == IMAGE {
                let (offset, len) = ((data >> 12) as usize, (data & 0xFFF) as usize);
                Data::Url(std::str::from_utf8(&urls[offset..offset + len]).unwrap().to_string())
            } else {
                Data::Int(data)
            };
            Span(start, end, t, data)
        })
        .collect();
    Blob {
        text,
        ascii: header >> 24 & 1 == 1,
        spans,
    }
}

/// The comment's blob, which must exist.
fn blob(html: &str) -> Blob {
    read(&parse(html).unwrap_or_else(|| panic!("no blob for {html:?}")))
}

/// The comment's text, with no spans.
fn plain(html: &str) -> String {
    let b = blob(html);
    assert_eq!(b.spans, vec![], "spans for {html:?}");
    b.text
}

/// The comment's text and spans, in the blob's order: start ascending, end descending.
fn spanned(html: &str) -> (String, Vec<Span>) {
    let b = blob(html);
    (b.text, b.spans)
}

// ── nothing ──────────────────────────────────────────────────────────────

/// A story with no text has no blob.
#[test]
fn nothing_empty_is_none() {
    assert!(parse("").is_none());
}

/// Words with no tag, entity or newline are shown as they are, so need no blob.
#[test]
fn nothing_plain_words_are_none() {
    assert!(parse("just words, (c) 2026 and a *star*").is_none());
}

/// A `<` that opens no tag is a word, and words read back unchanged need no blob.
#[test]
fn nothing_unchanged_text_is_none() {
    assert!(parse("a < b").is_none());
}

/// HTML with no words is a blob of no text, never `None`, which would show its tags.
#[test]
fn nothing_no_words_is_empty_text() {
    assert_eq!(plain("<p>"), "");
}

// ── paragraphs ───────────────────────────────────────────────────────────

/// The site opens each paragraph after the first with a bare `<p>` and closes none.
#[test]
fn p_opens_a_paragraph() {
    assert_eq!(plain("one<p>two<p>three"), "one\n\ntwo\n\nthree");
}

/// A comment opening on a `<p>`, as one starting with code does, has no gap before it.
#[test]
fn p_at_the_start_adds_nothing() {
    assert_eq!(plain("<p>one"), "one");
}

/// Two `<p>` with nothing between are one gap.
#[test]
fn p_empty_paragraph_adds_no_second_gap() {
    assert_eq!(plain("one<p><p>two"), "one\n\ntwo");
}

/// A closing `</p>`, which the site does not write but HTML allows, ends nothing more.
#[test]
fn p_closed_is_the_same_paragraph() {
    assert_eq!(plain("<p>one</p><p>two</p>"), "one\n\ntwo");
}

/// A newline in the HTML is a space between words, as markdown's soft break is.
#[test]
fn whitespace_newline_is_a_space() {
    assert_eq!(plain("a\nb"), "a b");
}

/// A run of spaces and newlines is one space.
#[test]
fn whitespace_runs_collapse() {
    assert_eq!(plain("a  \n  b<p>c"), "a b\n\nc");
}

/// Space at a paragraph's start or end is no text.
#[test]
fn whitespace_at_a_paragraph_edge_is_dropped() {
    assert_eq!(plain("<p> one <p> two "), "one\n\ntwo");
}

/// A `<br>` is a newline within the paragraph.
#[test]
fn br_is_a_newline() {
    assert_eq!(plain("one<br>two"), "one\ntwo");
}

// ── entities ─────────────────────────────────────────────────────────────

/// The site writes an apostrophe and a slash as hex entities.
#[test]
fn entities_hex_are_decoded() {
    assert_eq!(plain("I didn&#x27;t &#x2F; won&#x27;t"), "I didn't / won't");
}

/// The named entities read as their characters, a `<` among them staying a word.
#[test]
fn entities_named_are_decoded() {
    assert_eq!(plain("&quot;a&quot; &amp; b &lt; c"), "\"a\" & b < c");
}

/// An escaped tag is words, since entities are decoded after the tags are read.
#[test]
fn entities_escaped_tag_stays_words() {
    assert_eq!(plain("&lt;i&gt;not italic&lt;/i&gt;"), "<i>not italic</i>");
}

/// An entity past the basic plane is two UTF-16 units, so a span after it starts at 3, and the
/// text is flagged as not ASCII.
#[test]
fn entities_past_the_basic_plane_count_two_units() {
    let b = blob("&#x1F600; <i>x</i>");
    assert_eq!(b.text, "😀 x");
    assert_eq!(b.spans, vec![span(3, 4, ITALIC)]);
    assert!(!b.ascii);
}

/// ASCII text is flagged so, letting the reader skip its own scan.
#[test]
fn entities_ascii_text_is_flagged() {
    assert!(blob("I didn&#x27;t").ascii);
}

// ── quotes ───────────────────────────────────────────────────────────────

/// A paragraph opening on `>` is a quote, its mark and the space after it no text.
#[test]
fn quote_paragraph_opening_on_a_mark() {
    assert_eq!(
        spanned("&gt; On October 17, 2026"),
        ("On October 17, 2026".to_string(), vec![quote(0, 19, 0)])
    );
}

/// The quote is its paragraph alone.
#[test]
fn quote_is_its_paragraph_alone() {
    assert_eq!(
        spanned("before<p>&gt; quoted<p>after"),
        ("before\n\nquoted\n\nafter".to_string(), vec![quote(8, 14, 0)])
    );
}

/// Two quoted paragraphs are two quotes.
#[test]
fn quote_each_paragraph_its_own() {
    assert_eq!(
        spanned("&gt; one<p>&gt; two"),
        ("one\n\ntwo".to_string(), vec![quote(0, 3, 0), quote(5, 8, 0)])
    );
}

/// The mark needs no space after it.
#[test]
fn quote_mark_without_a_space() {
    assert_eq!(spanned("&gt;implying"), ("implying".to_string(), vec![quote(0, 8, 0)]));
}

/// Two marks are a quote within a quote, the inner one a level deeper.
#[test]
fn quote_two_marks_nest() {
    assert_eq!(
        spanned("&gt;&gt; nested"),
        ("nested".to_string(), vec![quote(0, 6, 1), quote(0, 6, 0)])
    );
}

/// Marks with a space between them nest as two marks together do.
#[test]
fn quote_spaced_marks_nest() {
    assert_eq!(
        spanned("&gt; &gt; nested"),
        ("nested".to_string(), vec![quote(0, 6, 1), quote(0, 6, 0)])
    );
}

/// Space before the mark is HTML's, collapsed and meaning nothing, so the paragraph is still a
/// quote however much of it there is.
#[test]
fn quote_space_before_the_mark() {
    assert_eq!(spanned("    &gt; x"), ("x".to_string(), vec![quote(0, 1, 0)]));
}

/// A mark opening a code block's line is code, never a quote.
#[test]
fn quote_mark_in_code_is_code() {
    assert_eq!(
        spanned("<pre><code>  &gt; quoted</code></pre>"),
        ("  > quoted".to_string(), vec![span(0, 10, CODE_BLOCK)])
    );
}

/// A mark inside a paragraph is a word.
#[test]
fn quote_mark_mid_paragraph_is_a_word() {
    assert_eq!(plain("a &gt; b"), "a > b");
}

/// A quote keeps the spans inside it.
#[test]
fn quote_holds_its_emphasis() {
    assert_eq!(
        spanned("&gt; a <i>b</i>"),
        ("a b".to_string(), vec![quote(0, 3, 0), span(2, 3, ITALIC)])
    );
}

// ── links ────────────────────────────────────────────────────────────────

/// The address is the site's, its slashes written as entities in the attribute, and words that
/// name the domain get no suffix.
#[test]
fn a_keeps_its_address_decoded() {
    let html = "<a href=\"https:&#x2F;&#x2F;github.com&#x2F;mitmproxy&#x2F;mitmproxy&#x2F;pull&#x2F;8503\" rel=\"nofollow\">\
                https:&#x2F;&#x2F;github.com&#x2F;mitmproxy&#x2F;mitmproxy&#x2F;pull&#x2F;8503</a> adds the AAA root path.";
    assert_eq!(
        spanned(html),
        (
            "https://github.com/mitmproxy/mitmproxy/pull/8503 adds the AAA root path.".to_string(),
            link(0, 48, "https://github.com/mitmproxy/mitmproxy/pull/8503").into()
        )
    );
}

/// The site shortens a long address's words with an ellipsis; the link still goes to the whole.
#[test]
fn a_shortened_words_keep_the_whole_address() {
    let html = "<a href=\"https:&#x2F;&#x2F;en.wikipedia.org&#x2F;wiki&#x2F;EMudhra#2026_compromise\">\
                https:&#x2F;&#x2F;en.wikipedia.org&#x2F;wiki&#x2F;EMudhra#2026_comp...</a>";
    assert_eq!(
        spanned(html),
        (
            "https://en.wikipedia.org/wiki/EMudhra#2026_comp...".to_string(),
            link(0, 50, "https://en.wikipedia.org/wiki/EMudhra#2026_compromise").into()
        )
    );
}

/// Words that don't name the domain are followed by it.
#[test]
fn a_words_get_the_domain() {
    assert_eq!(
        spanned("<a href=\"https://boards.4chan.org/g/thread/200#p201\">post</a>"),
        (
            "post (boards.4chan.org)".to_string(),
            link(0, 4, "https://boards.4chan.org/g/thread/200#p201").into()
        )
    );
}

/// The domain loses its `www.`, and the spaces around the link stay outside it.
#[test]
fn a_mid_sentence() {
    assert_eq!(
        spanned("see <a href=\"https://www.example.com/a\">the docs</a> now"),
        ("see the docs (example.com) now".to_string(), link(4, 12, "https://www.example.com/a").into())
    );
}

/// The address is resolved as comrak resolves one: a short YouTube link is the watch page.
#[test]
fn a_address_is_resolved() {
    assert_eq!(
        spanned("<a href=\"https://youtu.be/abc\">https://youtu.be/abc</a>"),
        (
            "https://youtu.be/abc (youtube.com)".to_string(),
            link(0, 20, "https://www.youtube.com/watch?v=abc").into()
        )
    );
}

/// An address in single quotes reads the same.
#[test]
fn a_single_quoted_address() {
    assert_eq!(
        spanned("<a href='https://example.com/'>x</a>"),
        ("x (example.com)".to_string(), link(0, 1, "https://example.com/").into())
    );
}

/// An anchor with no address is its words.
#[test]
fn a_without_an_address_is_words() {
    assert_eq!(plain("<a name=\"top\">x</a> y"), "x y");
}

/// A link whose words are its own address, to a picture, is the picture, as comrak's autolink.
#[test]
fn a_to_a_picture_named_by_its_address_is_the_picture() {
    assert_eq!(
        spanned("<a href=\"https://i.imgur.com/abc.png\">https://i.imgur.com/abc.png</a>"),
        ("\u{1}".to_string(), vec![url(0, 1, IMAGE, "https://i.imgur.com/abc.png")])
    );
}

/// The picture stands apart from the words around it.
#[test]
fn a_to_a_picture_mid_sentence() {
    assert_eq!(
        spanned("look <a href=\"https://i.imgur.com/abc.png\">https://i.imgur.com/abc.png</a> here"),
        ("look \n\n\u{1}\n\n here".to_string(), vec![url(7, 8, IMAGE, "https://i.imgur.com/abc.png")])
    );
}

/// A link to a picture with words of its own stays a link.
#[test]
fn a_to_a_picture_with_words_is_a_link() {
    assert_eq!(
        spanned("<a href=\"https://i.imgur.com/abc.png\">a pic</a>"),
        ("a pic (i.imgur.com)".to_string(), link(0, 5, "https://i.imgur.com/abc.png").into())
    );
}

// ── emphasis ─────────────────────────────────────────────────────────────

/// The site's one emphasis is italics.
#[test]
fn i_is_italic() {
    assert_eq!(
        spanned("i <i>havent</i> updated"),
        ("i havent updated".to_string(), vec![span(2, 8, ITALIC)])
    );
}

/// `<em>` is italic too.
#[test]
fn em_is_italic() {
    assert_eq!(spanned("<em>x</em>"), ("x".to_string(), vec![span(0, 1, ITALIC)]));
}

/// `<b>` and `<strong>` are bold.
#[test]
fn b_and_strong_are_bold() {
    assert_eq!(
        spanned("a <b>bold</b> <strong>b</strong>"),
        ("a bold b".to_string(), vec![span(2, 6, BOLD), span(7, 8, BOLD)])
    );
}

/// A tag's name reads in any case.
#[test]
fn tags_read_in_any_case() {
    assert_eq!(spanned("<I>x</I>"), ("x".to_string(), vec![span(0, 1, ITALIC)]));
}

// ── code ─────────────────────────────────────────────────────────────────

/// Code keeps its lines and their indent, a paragraph's gap before it.
#[test]
fn pre_keeps_its_lines() {
    assert_eq!(
        spanned("logs:<p><pre><code>  Audit Logs\n  Oct 21, 2026</code></pre>"),
        ("logs:\n\n  Audit Logs\n  Oct 21, 2026".to_string(), vec![span(7, 34, CODE_BLOCK)])
    );
}

/// The block's own last newline is no text, and the words after it are a new paragraph.
#[test]
fn pre_ending_on_a_newline() {
    assert_eq!(
        spanned("<pre><code>  x = 1\n</code></pre>\nafter"),
        ("  x = 1\n\nafter".to_string(), vec![span(0, 7, CODE_BLOCK)])
    );
}

/// Markdown's characters in code are code.
#[test]
fn pre_markdown_characters_are_code() {
    assert_eq!(
        spanned("<pre><code>  a *b* [c]</code></pre>"),
        ("  a *b* [c]".to_string(), vec![span(0, 11, CODE_BLOCK)])
    );
}

/// Entities in code are decoded.
#[test]
fn pre_entities_are_decoded() {
    assert_eq!(
        spanned("<pre><code>a &lt; b</code></pre>"),
        ("a < b".to_string(), vec![span(0, 5, CODE_BLOCK)])
    );
}

/// Code is as written: no typographic symbols, no collapsed spaces.
#[test]
fn pre_has_no_typography() {
    assert_eq!(
        spanned("<pre><code>(c)  +-</code></pre>"),
        ("(c)  +-".to_string(), vec![span(0, 7, CODE_BLOCK)])
    );
}

// ── words ────────────────────────────────────────────────────────────────

/// Markdown's characters are words: nothing reads them, so none is escaped.
#[test]
fn words_markdown_characters_stay() {
    assert_eq!(
        spanned("<i>x</i> a *b* _c_ `d` ~e"),
        ("x a *b* _c_ `d` ~e".to_string(), vec![span(0, 1, ITALIC)])
    );
}

/// Tags no rule reads, 4chan's greentext span and strike among them, leave their words.
#[test]
fn words_unread_tags_keep_their_words() {
    assert_eq!(plain("<span class=\"quote\">a</span> <s>b</s>"), "a b");
}

/// A comment is no text.
#[test]
fn words_comments_are_dropped() {
    assert_eq!(plain("a<!-- hidden -->b"), "ab");
}

/// The four symbol codes are their symbols, as comrak's blob writes them.
#[test]
fn words_typographic_symbols() {
    let b = blob("(c) 2026 +- 1<p>x");
    assert_eq!(b.text, "\u{a9} 2026 \u{b1} 1\n\nx");
    assert!(!b.ascii);
}

/// Typed curly quotes and dashes are their ASCII forms.
#[test]
fn words_prefer_ascii() {
    assert_eq!(plain("it&#x2019;s &#x201C;quoted&#x201D; &#x2014; dash&#x2026;"), "it's \"quoted\" - dash...");
}

// ── whole ────────────────────────────────────────────────────────────────

/// A real comment, as Algolia returns it: entities, a paragraph and a link whose words are its
/// address.
#[test]
fn whole_real_comment() {
    let html = "Ha, I didn&#x27;t expect this to show up on HN!<p>Release is out now, including a blog post at \
                <a href=\"https:&#x2F;&#x2F;www.mitmproxy.org&#x2F;posts&#x2F;releases&#x2F;mitmproxy-12.3&#x2F;\" rel=\"nofollow\">\
                https:&#x2F;&#x2F;www.mitmproxy.org&#x2F;posts&#x2F;releases&#x2F;mitmproxy-12.3&#x2F;</a>. :)";
    assert_eq!(
        spanned(html),
        (
            "Ha, I didn't expect this to show up on HN!\n\nRelease is out now, including a blog post at \
             https://www.mitmproxy.org/posts/releases/mitmproxy-12.3/. :)"
                .to_string(),
            link(89, 145, "https://www.mitmproxy.org/posts/releases/mitmproxy-12.3/").into()
        )
    );
}
