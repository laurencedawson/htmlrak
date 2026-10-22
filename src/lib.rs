//! A site's comment HTML read straight into syncdown's span blob, with no markdown between.

pub mod benchmarks;
mod blob;
mod entities;
mod image_url;
mod scan;
mod text;
mod url;
mod walk;

/// Reads `html` into a blob: `[text_len:4][span_count:3|flags:1][text][pad][spans][url_data]`,
/// positions in UTF-16 units, as comrak's blob for the same content.
///
/// The same contract as syncdown's `parse`: `None` for empty input, for text with no `<`, `&` or
/// newline, which is shown as it is, and for input whose text comes out unchanged with no spans.
pub fn parse(html: &str) -> Option<Vec<u8>> {
    if !html.bytes().any(|b| matches!(b, b'<' | b'&' | b'\n')) {
        return None;
    }
    let out = walk::walk(html);
    if out.spans.is_empty() && out.text() == html {
        None
    } else {
        Some(out.into_blob())
    }
}

#[cfg(test)]
mod tests;
