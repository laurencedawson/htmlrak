//! Low-level HTML scanning primitives: whitespace, comments, tag attributes.
//!
//! From mdream 2.0.3's `scan.rs` (MIT, see `licenses/mdream.txt`), cut to a whole string read at
//! once: the chunk carries, discarded-byte scans and attribute budgets are left behind, and only
//! the attributes asked for are kept.

/// Whitespace check optimized for the hot character loop.
/// Uses a 33-bit bitmap: space(32), CR(13), LF(10), TAB(9).
#[inline(always)]
pub(crate) fn is_whitespace(c: u8) -> bool {
  if c > 32 {
    return false;
  }
  // Bitmap: bit 9 (tab), bit 10 (LF), bit 12 (FF), bit 13 (CR), bit 32 (space)
  const MASK: u64 = (1u64 << 9) | (1u64 << 10) | (1u64 << 12) | (1u64 << 13) | (1u64 << 32);
  (MASK >> c) & 1 == 1
}

const DASH_CHAR: u8 = b'-';
const GT_CHAR: u8 = b'>';
const EXCLAMATION_CHAR: u8 = b'!';
const SLASH_CHAR: u8 = b'/';
const EQUALS_CHAR: u8 = b'=';
const QUOTE_CHAR: u8 = b'"';
const APOS_CHAR: u8 = b'\'';

pub(crate) struct CommentResult {
  pub(crate) complete: bool,
  pub(crate) new_position: usize,
}

/// A comment, doctype or bogus comment opening at `position` on `<!`.
pub(crate) fn process_comment_or_doctype(html_chunk: &str, position: usize) -> CommentResult {
  let mut i = position;
  let bytes = html_chunk.as_bytes();
  let chunk_length = bytes.len();

  if i + 3 < chunk_length && bytes[i + 2] == DASH_CHAR && bytes[i + 3] == DASH_CHAR {
    i += 4;
    // `-->` is not the only terminator. `<!-->` and `<!--->` close straight out
    // of the comment start states, and a run of two or more dashes also closes
    // on `!>`. Matching `-->` alone leaves those comments open, so the scan
    // runs to end of chunk and every byte after them is discarded.
    if i < chunk_length && bytes[i] == GT_CHAR {
      return CommentResult {
        complete: true,
        new_position: i + 1,
      };
    }
    if i + 1 < chunk_length && bytes[i] == DASH_CHAR && bytes[i + 1] == GT_CHAR {
      return CommentResult {
        complete: true,
        new_position: i + 2,
      };
    }
    while i + 2 < chunk_length {
      // Scanning for the dash on its own keeps this loop vectorizable; the
      // three-byte window is only formed once a dash is found.
      while i + 2 < chunk_length && bytes[i] != DASH_CHAR {
        i += 1;
      }
      if i + 2 >= chunk_length {
        break;
      }
      if bytes[i + 1] == DASH_CHAR {
        let after_dashes = bytes[i + 2];
        if after_dashes == GT_CHAR {
          return CommentResult {
            complete: true,
            new_position: i + 3,
          };
        }
        if after_dashes == EXCLAMATION_CHAR && i + 3 < chunk_length && bytes[i + 3] == GT_CHAR {
          return CommentResult {
            complete: true,
            new_position: i + 4,
          };
        }
      }
      i += 1;
    }
    CommentResult {
      complete: false,
      new_position: position,
    }
  } else {
    i += 2;
    while i < chunk_length {
      if bytes[i] == GT_CHAR {
        i += 1;
        return CommentResult {
          complete: true,
          new_position: i,
        };
      }
      i += 1;
    }
    CommentResult {
      complete: false,
      new_position: i,
    }
  }
}

/// Outcome of scanning a start tag's attribute region.
pub(crate) struct TagScan {
  pub(crate) complete: bool,
  pub(crate) new_position: usize,
}

/// Where the scan sits within one `name="value"` triple. A quoted value is run to its delimiter
/// whole, so it needs no state of its own.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
  Gap,
  Name,
  AfterName,
  BeforeValue,
  UnquotedValue,
}

/// Walk a start tag's attributes from `position` to its `>`, handing each one `wanted` names to
/// `take` as its raw name and its value, entities still in it. A `>` inside a quoted value does
/// not end the tag; the first of a name given twice is the one kept.
pub(crate) fn scan_tag<'a>(
  html: &'a str,
  position: usize,
  wanted: &[&str],
  mut take: impl FnMut(&'a str, &'a str),
) -> TagScan {
  let bytes = html.as_bytes();
  let mut taken = 0u8;
  let mut push = |name: &'a str, value: &'a str| {
    if let Some(i) = wanted.iter().position(|w| w.eq_ignore_ascii_case(name))
      && taken & (1 << i) == 0
    {
      taken |= 1 << i;
      take(name, value);
    }
  };
  let mut state = State::Gap;
  let (mut name_start, mut name_end, mut value_start) = (0, 0, 0);
  let mut i = position;

  while i < bytes.len() {
    let c = bytes[i];

    if state != State::UnquotedValue && c == SLASH_CHAR && bytes.get(i + 1) == Some(&GT_CHAR) {
      finish(html, state, i, name_start, name_end, value_start, &mut push);
      return TagScan {
        complete: true,
        new_position: i + 2,
      };
    }
    if c == GT_CHAR {
      finish(html, state, i, name_start, name_end, value_start, &mut push);
      return TagScan {
        complete: true,
        new_position: i + 1,
      };
    }

    // Run to the closing quote without re-entering the state dispatch.
    if state == State::BeforeValue && (c == QUOTE_CHAR || c == APOS_CHAR) {
      let value_start = i + 1;
      match bytes[value_start..].iter().position(|&b| b == c) {
        Some(offset) => {
          push(&html[name_start..name_end], &html[value_start..value_start + offset]);
          state = State::Gap;
          i = value_start + offset + 1;
          continue;
        }
        None => break,
      }
    }

    match state {
      State::Gap => {
        if !is_whitespace(c) {
          state = State::Name;
          name_start = i;
        }
      }
      State::Name => {
        if c == EQUALS_CHAR || is_whitespace(c) {
          name_end = i;
          state = if c == EQUALS_CHAR {
            State::BeforeValue
          } else {
            State::AfterName
          };
        }
      }
      State::AfterName => {
        if c == EQUALS_CHAR {
          state = State::BeforeValue;
        } else if !is_whitespace(c) {
          push(&html[name_start..name_end], "");
          state = State::Name;
          name_start = i;
        }
      }
      State::BeforeValue => {
        if !is_whitespace(c) {
          state = State::UnquotedValue;
          value_start = i;
        }
      }
      State::UnquotedValue => {
        if is_whitespace(c) {
          push(&html[name_start..name_end], &html[value_start..i]);
          state = State::Gap;
        }
      }
    }
    i += 1;
  }

  TagScan {
    complete: false,
    new_position: bytes.len(),
  }
}

/// Take the attribute still open when the tag ended at `end`.
#[inline]
fn finish<'a>(
  html: &'a str,
  state: State,
  end: usize,
  name_start: usize,
  name_end: usize,
  value_start: usize,
  push: &mut impl FnMut(&'a str, &'a str),
) {
  match state {
    State::Name => push(&html[name_start..end], ""),
    State::AfterName | State::BeforeValue => push(&html[name_start..name_end], ""),
    State::UnquotedValue => push(&html[name_start..name_end], &html[value_start..end]),
    State::Gap => {}
  }
}
