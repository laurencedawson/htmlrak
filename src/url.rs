//! From comrak's `parser/url.rs` in syncdown (BSD-2-Clause, see `licenses/comrak.txt`).

use memchr::memmem::Finder;
use std::borrow::Cow;
use std::ops::Deref;
use std::sync::LazyLock;

// These gates run once per rendered link. `str::contains`/`find` with a
// multi-byte pattern rebuilds its two-way searcher on every call, which
// dominates resolve_url in profiles; a precompiled Finder pays that once.
static API_V: LazyLock<Finder<'static>> = LazyLock::new(|| Finder::new("/api/v"));
static PICTRS_IMAGE: LazyLock<Finder<'static>> = LazyLock::new(|| Finder::new("/pictrs/image/"));
static SCHEME_SEP: LazyLock<Finder<'static>> = LazyLock::new(|| Finder::new("://"));

/// The scheme an app writes its own links under. Such a link names a place in
/// the app, not an address: it is never resolved and has no host to show.
const APP_LINK: &str = "app-link://";

/// A URL that has been through [`resolve_url`]. Its only constructor is
/// `resolve_url`, so holding one is proof the URL was finalized exactly once.
/// The blob writer accepts `&ResolvedUrl`, never a raw `&str`, which makes
/// "every emitted URL is resolved, and resolved only once" a compile-time
/// invariant rather than a convention. Derefs to `str`, so it passes straight
/// into the `&str` predicate/text helpers with no ceremony.
#[derive(Debug)]
pub struct ResolvedUrl<'a>(Cow<'a, str>);

impl Deref for ResolvedUrl<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ResolvedUrl<'_> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Finalize a URL for display: resolve it to its real target (unwrap proxies/redirects,
/// expand short URLs) and, for Lemmy pict-rs images, rewrite to a thumbnail preview.
/// The single URL finalizer; the [`ResolvedUrl`] it returns is the only thing the blob
/// writer will emit, so a URL is resolved here once and never re-processed. Note this
/// thumbnails pict-rs URLs even on links, not just inline images; harmless because the
/// host strips the query for full-res (see image_url::pictrs_preview).
pub fn resolve_url(url: &str) -> ResolvedUrl<'_> {
    if url.starts_with(APP_LINK) {
        return ResolvedUrl(Cow::Borrowed(url));
    }
    let resolved = resolve_target(url);
    let finalized = match pictrs_preview(&resolved) {
        Cow::Owned(s) => Cow::Owned(s),
        Cow::Borrowed(_) => resolved,
    };
    ResolvedUrl(finalized)
}

/// Resolve a URL to its real target: proxy unwrapping, redirect unwrapping, short-URL
/// and mobile-URL expansion. Returns `Cow::Borrowed` when nothing changed (zero-copy).
fn resolve_target(url: &str) -> Cow<'_, str> {
    let url = unwrap_proxy(url);
    let rest = match strip_scheme(&url) {
        Some(r) => r,
        None => return url,
    };
    let host = memchr::memchr(b'/', rest.as_bytes()).map_or(rest, |i| &rest[..i]);

    if host.ends_with("duckduckgo.com") {
        if let Some(u) = unwrap_redirect(&url, "external-content.duckduckgo.com/iu/", "u") {
            return Cow::Owned(u);
        }
    } else if host.ends_with("google.com") {
        if let Some(u) = unwrap_redirect(&url, "www.google.com/url?", "q")
            .or_else(|| unwrap_redirect(&url, "google.com/url?", "q"))
            .or_else(|| unwrap_google_amp(&url))
        {
            return Cow::Owned(u);
        }
    } else if host.ends_with("youtube.com") {
        if let Some(u) = unwrap_redirect(&url, "www.youtube.com/redirect?", "q")
            .or_else(|| unwrap_redirect(&url, "youtube.com/redirect?", "q"))
            .or_else(|| normalize_mobile_youtube(&url))
        {
            return Cow::Owned(u);
        }
    } else if host.ends_with("facebook.com") {
        if let Some(u) = unwrap_redirect(&url, "l.facebook.com/l.php?", "u") {
            return Cow::Owned(u);
        }
    } else if host.ends_with("discordapp.net") {
        if let Some(u) = unwrap_discord_image(&url) {
            return Cow::Owned(u);
        }
    } else if host.ends_with("skimresources.com") {
        if let Some(u) = unwrap_redirect(&url, "go.skimresources.com/", "url") {
            return Cow::Owned(u);
        }
    } else if host.ends_with("vger.to") {
        if let Some(u) = unwrap_path_prefix(&url, "vger.to/") {
            return Cow::Owned(u);
        }
    } else if host.eq_ignore_ascii_case("youtu.be")
        && let Some(u) = expand_youtube_short(&url)
    {
        return Cow::Owned(u);
    }

    url
}

/// Width passed to pict-rs `thumbnail` for inline image previews. Caps the in-body
/// download; the full-size original is one tap away (the host's image viewer strips
/// the query). Fixed because the parser has no display width, so an image shown wider
/// than this upscales slightly.
const PICTRS_PREVIEW_WIDTH: u32 = 400;

/// If `url` is a Lemmy pict-rs image, rewrite it to request a server-side `thumbnail`
/// resize in webp, the only processing Lemmy honors on image URLs (`crop`/`resize`
/// are silently ignored). Any existing query is dropped. Animated (`.gif`) and video
/// formats are left untouched because the webp transcode strips animation or reduces a
/// video to a still frame. Non-pict-rs URLs pass through unchanged (zero-copy).
fn pictrs_preview(url: &str) -> Cow<'_, str> {
    if PICTRS_IMAGE.find(url.as_bytes()).is_none() {
        return Cow::Borrowed(url);
    }
    let non_static = path_ext(url).is_some_and(|e| {
        e.eq_ignore_ascii_case("gif")
            || crate::image_url::VIDEO_EXTENSIONS
                .iter()
                .any(|v| e.eq_ignore_ascii_case(v))
    });
    if non_static {
        return Cow::Borrowed(url);
    }
    let path = url.split(['?', '#']).next().unwrap_or(url);
    Cow::Owned(format!(
        "{path}?thumbnail={PICTRS_PREVIEW_WIDTH}&format=webp"
    ))
}

fn unwrap_redirect(url: &str, prefix: &str, param: &str) -> Option<String> {
    let rest = strip_scheme(url)?;
    if !rest.starts_with(prefix) {
        return None;
    }
    let dest = query_param(url, param)?;
    if dest.is_empty() || !dest.starts_with("http") {
        return None;
    }
    Some(dest)
}

fn unwrap_path_prefix(url: &str, prefix: &str) -> Option<String> {
    let rest = strip_scheme(url)?;
    let path = rest
        .strip_prefix(prefix)
        .or_else(|| rest.strip_prefix(&format!("www.{prefix}")))?;
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{path}"))
}

fn unwrap_google_amp(url: &str) -> Option<String> {
    let rest = strip_scheme(url)?;
    let path = rest
        .strip_prefix("www.google.com/amp/s/")
        .or_else(|| rest.strip_prefix("google.com/amp/s/"))?;
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{path}"))
}

fn unwrap_discord_image(url: &str) -> Option<String> {
    let rest = strip_scheme(url)?;
    if !(rest.starts_with("images-ext-") && rest.contains(".discordapp.net/external/")) {
        return None;
    }
    query_param(url, "url")
}

fn unwrap_proxy(url: &str) -> Cow<'_, str> {
    if API_V.find(url.as_bytes()).is_none() {
        return Cow::Borrowed(url);
    }
    if !(url.contains("/api/v3/image_proxy") || url.contains("/api/v4/image/proxy")) {
        return Cow::Borrowed(url);
    }
    if let Some(original) = query_param(url, "url") {
        return Cow::Owned(original);
    }
    Cow::Borrowed(url)
}

fn expand_youtube_short(url: &str) -> Option<String> {
    let rest = strip_scheme(url)?;
    let (host, path) = rest.split_once('/')?;
    if !host.eq_ignore_ascii_case("youtu.be") {
        return None;
    }
    if path.starts_with("watch") {
        let id = query_param(url, "v")?;
        if id.is_empty() {
            return None;
        }
        return Some(format!("https://www.youtube.com/watch?v={id}"));
    }
    let id = path.split(['?', '&', '#', '/']).next()?;
    if id.is_empty() {
        return None;
    }
    Some(format!("https://www.youtube.com/watch?v={id}"))
}

fn normalize_mobile_youtube(url: &str) -> Option<String> {
    let rest = strip_scheme(url)?;
    rest.strip_prefix("m.youtube.com/")
        .map(|path| format!("https://www.youtube.com/{path}"))
}

fn strip_scheme(url: &str) -> Option<&str> {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
}

/// The extension of a URL's last path segment, with any query/fragment stripped.
/// `"https://x/a/b.MP4?q=1"` -> `Some("MP4")`. Case preserved; callers compare
/// case-insensitively.
pub(crate) fn path_ext(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/')
        .next()?
        .rsplit_once('.')
        .map(|(_, ext)| ext)
}

/// Extract and percent-decode a query parameter. Decodes per RFC 3986, not
/// form-urlencoding: a raw `+` is a literal `+` (common in proxied file URLs),
/// never a space. Returns `None` for malformed percent sequences or a decoded
/// value containing whitespace/control chars — such a value can't be a usable
/// URL, and callers keep the wrapped URL instead, which still serves.
fn query_param(url: &str, param: &str) -> Option<String> {
    // Query = between the first '?' after the scheme separator and the first
    // '#': a '?' inside the fragment is not a query. Requiring the scheme
    // keeps the old Url::parse behaviour of rejecting scheme-less strings;
    // every caller feeds absolute http(s) URLs.
    let scheme_end = SCHEME_SEP.find(url.as_bytes())? + 3;
    let pre_fragment = url[scheme_end..].split('#').next().unwrap_or("");
    let query = &pre_fragment[pre_fragment.find('?')? + 1..];
    let raw = query.split('&').find_map(|kv| {
        kv.split_once('=')
            .filter(|(k, _)| *k == param)
            .map(|(_, v)| v)
    })?;
    if raw.is_empty() {
        return None;
    }
    let value = percent_decode(raw)?;
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    Some(value)
}

/// RFC 3986 percent-decoding: `%XX` becomes a byte, everything else (including
/// `+`) passes through literally. `None` on a malformed sequence or invalid
/// UTF-8, matching the percent-encoding-rfc3986 crate this replaced.
pub(crate) fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hi = (hex[0] as char).to_digit(16)?;
            let lo = (hex[1] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Extract the host component from a URL for display-suffix deduplication.
/// Returns `Cow::Borrowed` when the host is already lowercase ASCII — most
/// links in practice — avoiding an allocation per link render. Strips a
/// leading `www.` prefix; returns `None` when scheme or host is missing.
pub fn extract_domain(url: &str) -> Option<Cow<'_, str>> {
    if url.starts_with(APP_LINK) {
        return None;
    }
    let start = SCHEME_SEP.find(url.as_bytes())? + 3;
    let after = &url.as_bytes()[start..];
    let end = memchr::memchr(b'/', after)
        .or_else(|| memchr::memchr(b'?', after))
        .map_or(url.len(), |i| start + i);
    let mut host = &url[start..end];
    if host.starts_with("www.") {
        host = &host[4..];
    }
    if host.is_empty() {
        return None;
    }
    if host.bytes().all(|b| !b.is_ascii_uppercase()) {
        Some(Cow::Borrowed(host))
    } else {
        Some(Cow::Owned(host.to_ascii_lowercase()))
    }
}
