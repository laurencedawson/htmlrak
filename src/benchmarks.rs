//! Benchmark test data and helpers for blob writing.
//! Mirrors comrak's `benchmarks.rs` in syncdown: each corpus is comrak's written as the HTML a
//! site would send for it, so the two read the same comment side by side. htmlrak reads no
//! heading, list, rule, table or strike yet, so those tags are scanned and their words kept.

/// Plain text with no markdown syntax.
pub const PLAIN: &str = "Just some plain text with no markdown formatting at all.";

/// Single line with bold, italic, and inline code.
pub const SIMPLE: &str = "<b>bold</b> and <i>italic</i> and <code>code</code>";

/// Typical comment with headings, lists, blockquote, code block, and link.
pub const MEDIUM: &str = "\
<h1>Heading 1</h1>\n\
<p>Some <b>bold</b> text with <i>italic</i> and <code>inline code</code>.</p>\n\
<h2>Heading 2</h2>\n\
<p>&gt; A blockquote with <b>nested bold</b></p>\n\
<ul>\n<li>item one</li>\n<li>item two\n<ul>\n<li>nested item</li>\n</ul>\n</li>\n<li>item three</li>\n</ul>\n\
<ol>\n<li>first</li>\n<li>second</li>\n<li>third</li>\n</ol>\n\
<p><a href=\"https://example.com\">a link</a></p>\n\
<pre><code>code block\nwith lines\n</code></pre>\n\
<hr>\n\
<p><s>strikethrough</s> normal text</p>\n";

/// All heading levels, paragraphs with formatting, nested blockquotes, lists, code, table, hrule.
pub fn complex() -> String {
    let mut s = String::new();
    for i in 1..=6 {
        s.push_str(&format!("<h{i}>Heading {i}</h{i}>\n"));
    }
    for i in 0..10 {
        s.push_str(&format!(
            "<p>Paragraph {} with <b>bold</b>, <i>italic</i>, <s>strikethrough</s>, <code>code</code>, and a <a href=\"https://example.com/{}\">link</a>.</p>\n",
            i, i
        ));
    }
    s.push_str("<p>&gt; level 1</p>\n<p>&gt;&gt; level 2</p>\n<p>&gt;&gt;&gt; level 3</p>\n");
    s.push_str("<ul>\n");
    for i in 0..20 {
        s.push_str(&format!("<li>item {}</li>\n", i));
    }
    s.push_str("</ul>\n<ol>\n");
    for i in 1..=10 {
        s.push_str(&format!("<li>ordered {}</li>\n", i));
    }
    s.push_str("</ol>\n");
    s.push_str("<pre><code>public class Foo {\n    void bar() {}\n}\n</code></pre>\n");
    s.push_str("<table><tr><th>a</th><th>b</th><th>c</th></tr><tr><td>1</td><td>2</td><td>3</td></tr><tr><td>4</td><td>5</td><td>6</td></tr></table>\n");
    s.push_str("<hr>\n");
    s
}

/// 50 repeated bold/italic/code groups: stress test for inline parsing.
pub fn heavy_inline() -> String {
    let mut s = String::new();
    for i in 0..50 {
        s.push_str(&format!("<b>b{}</b> <i>i{}</i> <code>c{}</code> ", i, i, i));
    }
    s
}

/// 10 levels of nested blockquotes, a paragraph each, as Hacker News' posters write them.
pub fn deep_nesting() -> String {
    let mut s = String::from("&gt; level 1");
    for i in 2..=10 {
        s.push_str("<p>");
        for _ in 0..i {
            s.push_str("&gt;");
        }
        s.push_str(&format!(" level {}", i));
    }
    s
}

/// ~14K char document simulating a detailed post with 10 sections.
pub fn long_doc() -> String {
    let mut s = String::new();
    s.push_str("<h1>Introduction</h1>\n");
    s.push_str("<p>This is a long document with many sections, ");
    s.push_str("covering a wide range of <b>markdown</b> features.</p>\n");

    for section in 1..=10 {
        s.push_str(&format!("<h2>Section {}</h2>\n", section));
        for p in 0..5 {
            s.push_str(&format!(
                "<p>Paragraph {} with <b>bold text</b>, <i>italic text</i>, <s>strikethrough</s>, \
                 <code>inline code</code>, and a <a href=\"https://example.com/{}/{}\">link</a>. \
                 Some more text to pad this out to a realistic length, \
                 because real posts tend to have longer paragraphs.</p>\n",
                p, section, p
            ));
        }
        s.push_str(&format!("<p>&gt; A relevant quote for section {}</p>\n", section));
        s.push_str("<p>&gt;&gt; With nested context</p>\n");
        s.push_str("<ul>\n<li>point one</li>\n<li>point two</li>\n<li>point three\n<ul>\n<li>sub-point</li>\n</ul>\n</li>\n</ul>\n");
        s.push_str(&format!("<pre><code>code_example_{}()\n</code></pre>\n", section));
        s.push_str("<hr>\n");
    }

    s.push_str("<h2>Conclusion</h2>\n");
    s.push_str("<p>Final paragraph with <sup>superscript</sup> and <sub>subscript</sub> for good measure.</p>\n");
    s
}

/// ~14K chars of contraction-dense comment prose, its apostrophes and quotes written as Hacker
/// News writes them, as entities: the corpus to weight whenever a change touches text or entities.
pub fn prose_doc() -> String {
    let mut s = String::new();
    for i in 0..40 {
        if i > 0 {
            s.push_str("<p>");
        }
        s.push_str(&format!(
            "I don&#x27;t think that&#x27;s what they&#x27;re saying -- it&#x27;s more that you can&#x27;t \
             just &quot;quote the docs&quot; and call it done... The author&#x27;s point in \
             section {i} wasn&#x27;t about performance, it&#x27;s about correctness. \
             Here&#x27;s the <a href=\"https:&#x2F;&#x2F;example.com&#x2F;thread&#x2F;{i}\" rel=\"nofollow\">relevant thread</a> which \
             shouldn&#x27;t be ignored -- and yes, it&#x27;s been discussed before..."
        ));
    }
    s
}

/// Ten real comments from Algolia's Hacker News API, as fetched on 22 Oct 2026, each read alone.
pub fn hn_comments() -> Vec<&'static str> {
    include_str!("benchmarks/hn_comments.html").split("\n=====\n").collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn plain_returns_none() {
        assert!(parse(PLAIN).is_none(), "plain text should return None");
    }

    #[test]
    fn simple_produces_blob() {
        assert!(parse(SIMPLE).is_some(), "simple HTML should produce a blob");
    }

    #[test]
    fn medium_produces_blob() {
        assert!(parse(MEDIUM).is_some());
    }

    #[test]
    fn complex_produces_blob() {
        assert!(parse(&complex()).is_some());
    }

    #[test]
    fn heavy_inline_produces_blob() {
        assert!(parse(&heavy_inline()).is_some());
    }

    #[test]
    fn deep_nesting_produces_blob() {
        assert!(parse(&deep_nesting()).is_some());
    }

    #[test]
    fn long_doc_produces_blob() {
        let blob = parse(&long_doc());
        assert!(blob.is_some());
        let blob = blob.unwrap();
        // Blob should have header + text + spans
        assert!(blob.len() > 8, "blob too small: {} bytes", blob.len());
    }

    #[test]
    fn hn_comments_are_ten() {
        assert_eq!(hn_comments().len(), 10);
    }

    #[test]
    fn bench_all() {
        let inputs: Vec<(&str, String)> = vec![
            ("plain", PLAIN.to_string()),
            ("simple", SIMPLE.to_string()),
            ("medium", MEDIUM.to_string()),
            ("deep-nesting", deep_nesting()),
            ("heavy-inline", heavy_inline()),
            ("complex", complex()),
            ("long-doc", long_doc()),
            ("prose", prose_doc()),
        ];

        for (name, input) in &inputs {
            let trimmed = input.trim();

            // Warmup
            for _ in 0..100 {
                let _ = parse(trimmed);
            }

            let iterations = 500;
            let start = std::time::Instant::now();
            for _ in 0..iterations {
                let _ = parse(trimmed);
            }
            let elapsed = start.elapsed() / iterations;
            eprintln!(
                "{} ({} chars): {:.1} us",
                name,
                trimmed.len(),
                elapsed.as_nanos() as f64 / 1000.0
            );
        }

        // The real comments, the ten read in turn as one iteration
        let comments = hn_comments();
        for _ in 0..100 {
            comments.iter().for_each(|c| drop(parse(c)));
        }
        let iterations = 500;
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            comments.iter().for_each(|c| drop(parse(c)));
        }
        let elapsed = start.elapsed() / iterations;
        eprintln!(
            "hn-comments ({} chars): {:.1} us",
            comments.iter().map(|c| c.len()).sum::<usize>(),
            elapsed.as_nanos() as f64 / 1000.0
        );
    }
}
