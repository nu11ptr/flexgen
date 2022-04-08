use std::borrow::Cow;
use std::{cmp, slice};

use crate::Error;

const BLANK_IDENT: &[u8] = b"lank_!(";
const COMMENT_IDENT: &[u8] = b"omment_!(";
const DOC_BLOCK: &[u8] = b"[doc =";

const EMPTY_COMMENT: &str = "//";
const EMPTY_DOC_COMMENT: &str = "///";
const COMMENT_START: &str = "// ";
const DOC_COMMENT_START: &str = "/// ";

const MIN_BUFF_SIZE: usize = 128;

// In order to replace the markers there were a few options:
// 1. Create a full special purpose Rust lexer, replace the tokens we want as we go, write it back
// 2. Find the markers via regular string search, copy everything up to that point, replace, repeat
// 3. A hybrid of 1 and 2
//
// The problem with #1 is it is hugely overkill - we are only interested in 3 markers
// The problem with #2 is that it would find markers in strings and comments - likely not an issue, but it bothered me
// #3 is what is below - it does basic lexing of Rust comments and strings for the purposes of skipping them only. It
// understands just enough to do the job. The weird part is it literally searches inside all other constructs, but the
// probability of a false positive while low in comments and strings, is likely very close to zero anywhere else, so
// I think this is a good compromise. Regardless, the user will be advised to not use `_comment_!(` or `_blank_!(`
// anywhere in the source file other than where they want markers.
//
// It should also be noted we take some liberties with things like the doc block and assume it has been properly formatted
// and has exactly 1 space before the `=`, etc. Same with the macro markers and the parens - no white space allowed

struct CopyingCursor<'a> {
    start_idx: usize,
    curr_idx: usize,
    pub curr: u8,

    iter: slice::Iter<'a, u8>,
    source: &'a str,
    buffer: String,
}

impl<'a> CopyingCursor<'a> {
    fn new(source: &'a str) -> Option<Self> {
        // Better to be too large than not large enough
        let buffer = String::with_capacity(cmp::max(source.len() * 2, MIN_BUFF_SIZE));
        let mut iter = source.as_bytes().iter();

        iter.next().map(|&ch| Self {
            start_idx: 0,
            curr_idx: 0,
            curr: ch,
            iter,
            source,
            buffer,
        })
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        self.iter.next().map(|&ch| {
            self.curr_idx += 1;
            self.curr = ch;
            ch
        })
    }

    #[inline]
    fn copy_to_curr(&mut self) {
        self.copy_to_marker(self.curr_idx);
    }

    #[inline]
    fn copy_to_marker(&mut self, marker: usize) {
        if marker > self.start_idx {
            // Copy exclusive of marker position
            self.buffer.push_str(&self.source[self.start_idx..marker]);
            self.start_idx = marker;
        }
    }

    fn into_buffer(mut self) -> Cow<'a, str> {
        // We have done some work
        if self.start_idx > 0 {
            // Last write to ensure everything is copied
            self.copy_to_curr();

            self.buffer.shrink_to_fit();
            Cow::Owned(self.buffer)
        // We have done nothing - just return original str
        } else {
            Cow::Borrowed(self.source)
        }
    }

    fn skip_block_comment(&mut self) {
        enum State {
            InComment,
            MaybeStarting,
            MaybeEnding,
        }

        let mut nest_level = 1;
        let mut state = State::InComment;

        while let Some(ch) = self.next() {
            match (ch, state) {
                (b'*', State::InComment) => {
                    state = State::MaybeEnding;
                }
                (b'/', State::MaybeEnding) => {
                    nest_level -= 1;
                    if nest_level == 0 {
                        break;
                    }
                    state = State::InComment;
                }
                (b'*', State::MaybeStarting) => {
                    nest_level += 1;
                    state = State::InComment;
                }
                (b'/', State::InComment) => {
                    state = State::MaybeStarting;
                }
                (_, _) => {
                    state = State::InComment;
                }
            }
        }
    }

    fn try_skip_comment(&mut self) -> bool {
        match self.next() {
            // Line comment of some form (we don't care which
            Some(b'/') => {
                while let Some(ch) = self.next() {
                    if ch == b'\n' {
                        break;
                    }
                }

                true
            }
            // Block comment of some form (we don't care which)
            Some(b'*') => {
                self.skip_block_comment();
                true
            }
            // Not a comment or EOF, etc. - should be impossible in valid code
            _ => false,
        }
    }

    fn skip_string(&mut self) {
        let mut in_escape = false;

        while let Some(ch) = self.next() {
            match ch {
                b'"' if !in_escape => break,
                // Toggle every time we see a backslash
                b'\\' => in_escape = !in_escape,
                _ => {}
            }
        }
    }

    fn try_skip_raw_string(&mut self) -> bool {
        // First, match the entry sequence to the raw string and collect # of pads present
        let pads = match self.next() {
            Some(b'#') => {
                let mut pads = 1;

                while let Some(ch) = self.next() {
                    match ch {
                        b'#' => {
                            pads += 1;
                        }
                        b'"' => break,
                        // Not a raw string
                        _ => return false,
                    }
                }

                pads
            }
            Some(b'"') => 0,
            _ => return false,
        };

        #[derive(Clone, Copy)]
        enum State {
            InRawComment,
            MaybeEndingComment(i32),
        }

        let mut state = State::InRawComment;

        // Loop over the raw string looking for ending sequence and count pads until we have
        // the correct # of them
        while let Some(ch) = self.next() {
            match (ch, state) {
                (b'"', State::InRawComment) => state = State::MaybeEndingComment(0),
                (b'#', State::MaybeEndingComment(pads_seen)) => {
                    let pads_seen = pads_seen + 1;
                    if pads_seen == pads {
                        break;
                    }
                    state = State::MaybeEndingComment(pads_seen);
                }
                (_, _) => {
                    state = State::InRawComment;
                }
            }
        }

        true
    }

    fn try_match(&mut self, sl: &[u8]) -> bool {
        let mut iter = sl.iter();

        while let Some(ch) = iter.next() {
            if iter.next().is_none() {
                // This isn't great as it will reevaluate the last char - 'b' or 'c' in the main loop,
                // but since those aren't top level it will exit at the bottom of the main loop gracefully
                return false;
            }

            if self.curr != *ch {
                return false;
            }
        }

        // If we can exhaust the iterator then it must have matched
        true
    }

    fn process_blanks(buffer: &mut String, num: &str) -> Result<(), Error> {
        // Single blank line
        if num.is_empty() {
            // TODO: CRLF detection?
            buffer.push('\n');
        // Multiple blank lines
        } else {
            let num: syn::LitInt = syn::parse_str(num)?;
            let blanks: u32 = num.base10_parse()?;

            for _ in 0..blanks {
                // TODO: CRLF detection?
                buffer.push('\n');
            }
        }

        Ok(())
    }

    fn process_comments(buffer: &mut String, s: &str) -> Result<(), Error> {
        // Single blank comment
        if s.is_empty() {
            buffer.push_str(EMPTY_COMMENT);
            // TODO: CRLF detection?
            buffer.push('\n');
        // Multiple comments
        } else {
            let s: syn::LitStr = syn::parse_str(s)?;
            let comment = s.value();

            for line in comment.lines() {
                buffer.push_str(COMMENT_START);
                buffer.push_str(line);
                // TODO: CRLF detection?
                buffer.push('\n');
            }
        }

        Ok(())
    }

    fn try_match_replace<F>(&mut self, prefix: &[u8], f: F) -> Result<bool, Error>
    where
        F: FnOnce(&mut String, &str) -> Result<(), Error>,
    {
        // We already matched 2 chars before we got here
        let mark_start_ident = self.curr_idx - 2;

        if !self.try_match(prefix) {
            return Ok(false);
        }
        let mark_start_value = self.curr_idx;

        // Match until we find the end symbol
        while let Some(ch) = self.next() {
            if ch == b')' {
                // End of value (exclusive)
                let mark_end_value = self.curr_idx;

                if let Some(ch) = self.next() {
                    //
                    if ch != b';' {
                        break;
                    }

                    // Copy everything up until this marker
                    self.copy_to_marker(mark_start_ident);

                    // Parse and output
                    f(
                        &mut self.buffer,
                        &self.source[mark_start_value..mark_end_value],
                    )?;
                    return Ok(true);
                } else {
                    // Again another not great exit, but again ')' is not a top level char of interest
                    break;
                }
            }
        }

        Ok(false)
    }

    // NOTE: Tempting to merge with process_comments but that one is called from a closure
    // and can't have more params
    fn process_doc_block(buffer: &mut String, s: &str) -> Result<(), Error> {
        // Single blank comment
        if s.is_empty() {
            buffer.push_str(EMPTY_DOC_COMMENT);
            // TODO: CRLF detection?
            buffer.push('\n');
        // Multiple comments
        } else {
            let s: syn::LitStr = syn::parse_str(s)?;
            let comment = s.value();

            for line in comment.lines() {
                buffer.push_str(DOC_COMMENT_START);
                buffer.push_str(line);
                // TODO: CRLF detection?
                buffer.push('\n');
            }
        }

        Ok(())
    }

    fn try_doc_block_match_replace(&mut self) -> Result<bool, Error> {
        // We already matched 1 char before we got here
        let mark_start_ident = self.curr_idx - 1;

        if !self.try_match(DOC_BLOCK) {
            return Ok(false);
        }
        let mark_start_value = self.curr_idx;

        // Match until we find the end symbol
        while let Some(ch) = self.next() {
            if ch == b']' {
                // End of string (exclusive)
                let mark_end_value = self.curr_idx;

                // Copy everything up until this marker
                self.copy_to_marker(mark_start_ident);

                // Parse and output
                Self::process_doc_block(
                    &mut self.buffer,
                    &self.source[mark_start_value..mark_end_value],
                )?;
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[allow(dead_code)]
pub(crate) fn replace_markers(s: &str, replace_doc_blocks: bool) -> Result<Cow<str>, Error> {
    match CopyingCursor::new(s) {
        Some(mut cursor) => {
            loop {
                match cursor.curr {
                    // Possible raw string
                    b'r' => {
                        if !cursor.try_skip_raw_string() {
                            continue;
                        }
                    }
                    // Regular string
                    b'\"' => cursor.skip_string(),
                    // Possible comment
                    b'/' => {
                        if !cursor.try_skip_comment() {
                            continue;
                        }
                    }
                    // Possible special ident (_comment!_ or _blank!_)
                    b'_' => {
                        if cursor.next().is_none() {
                            break;
                        }

                        match cursor.curr {
                            // Possible blank marker
                            b'b' => {
                                if !cursor
                                    .try_match_replace(BLANK_IDENT, CopyingCursor::process_blanks)?
                                {
                                    continue;
                                }
                            }
                            // Possible comment marker
                            b'c' => {
                                if !cursor.try_match_replace(
                                    COMMENT_IDENT,
                                    CopyingCursor::process_comments,
                                )? {
                                    continue;
                                }
                            }
                            // Nothing we are interested in
                            _ => {
                                continue;
                            }
                        }
                    }
                    // Possible doc block
                    b'#' if replace_doc_blocks => {
                        if !cursor.try_doc_block_match_replace()? {
                            continue;
                        }
                    }
                    // Anything else
                    _ => {}
                }

                if cursor.next().is_none() {
                    break;
                }
            }

            Ok(cursor.into_buffer())
        }
        // Empty file
        None => Ok(Cow::Borrowed(s)),
    }
}
