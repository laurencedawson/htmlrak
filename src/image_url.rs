//! From comrak's `image_url.rs` in syncdown (BSD-2-Clause, see `licenses/comrak.txt`).
//!
//! Heuristic URL-to-image detection.
//!
//! Used by the blob renderer to convert bare autolinks pointing at image
//! resources into inline image embeds, and exposed to callers so they can
//! make the same decision at their own layer.

/// Strip `http://` / `https://`, returning the rest of the URL.
fn strip_scheme(url: &str) -> Option<&str> {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
}

/// Returns `true` if the URL points to an image (by known host, path pattern,
/// or file extension). Conservative: may return false for actual images with
/// unusual hosting, never true for non-image URLs.
pub fn is_image_url(url: &str) -> bool {
    let rest = match strip_scheme(url) {
        Some(r) => r,
        None => return false,
    };

    // Host/path shortcuts skip extension checking, so exclude video files explicitly:
    // pict-rs serves video under /pictrs/image/, and image hosts (imgur) serve mp4/gifv.
    if IMAGE_HOSTS.iter().any(|h| rest.starts_with(h)) {
        return !IMAGE_HOST_EXCLUDED.iter().any(|p| rest.starts_with(p)) && !is_video_url(rest);
    }

    if IMAGE_PATHS.iter().any(|p| rest.contains(p)) {
        return !is_video_url(rest);
    }

    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    for segment in path.split('/') {
        if let Some((_, ext)) = segment.rsplit_once('.')
            && IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
        {
            return true;
        }
    }

    if let Some(qs) = rest.split_once('?').map(|(_, q)| q) {
        for pair in qs.split('&') {
            let value = pair.split_once('=').map(|(_, v)| v).unwrap_or(pair);
            let v = value.split(['?', '#']).next().unwrap_or(value);
            if let Some((_, ext)) = v.rsplit_once('.')
                && IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
            {
                return true;
            }
        }
    }

    false
}

/// True if the URL's last path segment has a known video extension. Video files can
/// live on image hosts and pict-rs paths (where the image shortcuts would otherwise
/// treat them as images), or be embedded with `![](...)` image syntax.
pub(crate) fn is_video_url(url: &str) -> bool {
    crate::url::path_ext(url)
        .is_some_and(|ext| VIDEO_EXTENSIONS.iter().any(|v| ext.eq_ignore_ascii_case(v)))
}

const IMAGE_HOSTS: &[&str] = &[
    "i.redd.it/",
    "preview.redd.it/",
    "i.imgur.com/",
    "upload.wikimedia.org/",
    "s.yimg.com/",
    "encrypted-tbn0.gstatic.com/",
    "pbs.twimg.com/",
    "cdn.bsky.app/img/",
];

const IMAGE_HOST_EXCLUDED: &[&str] = &["i.imgur.com/a/", "i.imgur.com/gallery/"];

const IMAGE_PATHS: &[&str] = &[
    "/pictrs/image/",
    "/api/v3/image_proxy",
    "/api/v4/image/proxy",
];

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "avif", "svg", "ico", "tif", "tiff", "heic",
    "heif", "jfif",
];

/// Video extensions that can appear on image hosts / pict-rs paths. Used to keep
/// videos from being detected as images or thumbnailed into a still frame.
pub(crate) const VIDEO_EXTENSIONS: &[&str] =
    &["mp4", "webm", "mov", "m4v", "mkv", "avi", "gifv", "ogv"];
