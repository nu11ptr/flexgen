#![cfg(feature = "post_process")]

use std::borrow::Cow;
use std::{cmp, slice};

use crate::Error;

const BLANK_START: &[&[u8]] = &[b"lank_", b"!", b"("];
const BLANK_END: &[&[u8]] = &[b";"];
const COMMENT_START: &[&[u8]] = &[b"omment_", b"!", b"("];
const COMMENT_END: &[&[u8]] = &[b")", b";"];
const COMMENT_END2: &[&[u8]] = &[b";"];
const DOC_BLOCK_START: &[&[u8]] = &[b"[", b"doc", b"="];
const DOC_BLOCK_END: &[&[u8]] = &[b"]"];

const EMPTY_COMMENT: &str = "//";
const COMMENT: &str = "// ";
const DOC_COMMENT: &str = "///";
const LF_STR: &str = "\n";
const CRLF_STR: &str = "\r\n";

const CR: u8 = b'\r';
const LF: u8 = b'\n';

const MIN_BUFF_SIZE: usize = 128;

// In order to replace the markers there were a few options:
// 1. Create a full special purpose Rust lexer, replace the tokens we want as we go, write it back
// 2. Find the markers via regular string search, copy everything up to that point, replace, repeat
// 3. A hybrid of 1 and 2
//
// The problem with #1 is it is hugely overkill - we are only interested in 3 markers
// The problem with #2 is that it would find markers in strings and comments - likely not an issue, but it bothered me
// (and also we generalize the marker replacement code also for doc blocks, which someone could have commented out)
// #3 is what is below - it does basic lexing of Rust comments and strings for the purposes of skipping them only. It
// understands just enough to do the job. The weird part is it literally searches inside all other constructs, but the
// probability of a false positive while low in comments and strings, is likely very close to zero anywhere else, so
// I think this is a good compromise. Regardless, the user should be advised to not use `_comment_!(` or `_blank_!(`
// anywhere in the source file other than where they want markers.

struct CopyingCursor<'a> {
    start_idx: usize,
    curr_idx: usize,
    curr: u8,

    // We can iterate as if this were raw bytes since we are only matching ASCII. We preserve
    // any unicode, however, and copy it verbatim
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
    fn copy_to_marker(&mut self, marker: usize, new_start_idx: usize) {
        if marker > self.start_idx {
            // Copy exclusive of marker position
            self.buffer.push_str(&self.source[self.start_idx..marker]);
        }
        self.start_idx = new_start_idx;
    }

    fn into_buffer(mut self) -> Cow<'a, str> {
        // We have done some work
        if self.start_idx > 0 {
            // Last write to ensure everything is copied
            self.copy_to_marker(self.curr_idx + 1, self.curr_idx + 1);

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
            // Line comment of some form (we don't care which)
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
                b'\\' if !in_escape => in_escape = true,
                _ if in_escape => in_escape = false,
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
                (b'"', State::InRawComment) if pads == 0 => break,
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

    #[inline]
    fn skip_blank_param(&mut self) -> Result<(), Error> {
        while let Some(ch) = self.next() {
            if ch == b')' {
                return Ok(());
            }
        }

        // EOF
        Err(Error::BadSourceCode("Unexpected end of input".to_string()))
    }

    fn try_skip_string(&mut self) -> Result<Option<u8>, Error> {
        while let Some(ch) = self.next() {
            if Self::is_whitespace(ch) {
                continue;
            }

            return match ch {
                // Regular string
                b'"' => {
                    self.skip_string();
                    Ok(None)
                }
                // Raw string
                b'r' => {
                    if self.try_skip_raw_string() {
                        Ok(None)
                    } else {
                        Err(Error::BadSourceCode("Bad raw string".to_string()))
                    }
                }
                // Something else
                ch => Ok(Some(ch)),
            };
        }

        // EOF
        Err(Error::BadSourceCode("Unexpected end of input".to_string()))
    }

    // TODO: Was planning to match values here (but we only recognize ASCII atm):
    // https://github.com/rust-lang/rust/blob/38e0ae590caab982a4305da58a0a62385c2dd880/compiler/rustc_lexer/src/lib.rs#L245
    // We could switch back to UTF8 since we have been matching valid ASCII up to this point, but atm
    // any unicode whitespace will make it not match (not sure any code formatter preserves non-ASCII whitespace?)
    // For now, users should use NO whitespace and let the code formatters add any, if needed. I suspect
    // they will not add any non-ASCII whitespace on their own at min, but likely just ' ', '\n', and '\r'
    //
    // Code points we don't handle that we should (for future ref):
    // Code point 0x0085 == 0xC285
    // Code point 0x200E == 0xE2808E
    // Code point 0x200F == 0xE2808F
    // Code point 0x2028 == 0xE280A8
    // Code point 0x2029 == 0xE280A9
    #[inline]
    fn is_whitespace(ch: u8) -> bool {
        matches!(ch, b' ' | b'\n' | b'\r' | b'\t' | b'\x0b' | b'\x0c')
    }

    fn try_ws_matches(&mut self, slices: &[&[u8]], allow_whitespace_first: bool) -> bool {
        let mut allow_whitespace = allow_whitespace_first;

        'top: for &sl in slices {
            // Panic safety: it is pointless for us to pass in a blank slice, don't do that
            let first_ch = sl[0];

            while let Some(ch) = self.next() {
                // This is what we were looking for, now match the rest (if needed)
                if ch == first_ch {
                    // Panic safety: it is pointless for us to pass in a blank slice, don't do that
                    let remainder = &sl[1..];

                    if !remainder.is_empty() && !self.try_match(remainder) {
                        return false;
                    }
                    allow_whitespace = true;
                    continue 'top;
                } else if allow_whitespace && Self::is_whitespace(ch) {
                    // no op
                } else {
                    return false;
                }
            }

            // Premature EOF
            return false;
        }

        // If we can exhaust the iterator then they all must have matched
        true
    }

    fn try_match(&mut self, sl: &[u8]) -> bool {
        let iter = sl.iter();

        for &ch in iter {
            if self.next().is_none() {
                // This isn't great as it will reevaluate the last char - 'b' or 'c' in the main loop,
                // but since those aren't top level it will exit at the bottom of the main loop gracefully
                return false;
            }

            if self.curr != ch {
                return false;
            }
        }

        // If we can exhaust the iterator then it must have matched
        true
    }

    #[inline]
    fn detect_line_ending(&mut self) -> Option<&'static str> {
        match self.next() {
            Some(CR) => match self.next() {
                Some(LF) => Some(CRLF_STR),
                _ => None,
            },
            Some(LF) => Some(LF_STR),
            _ => None,
        }
    }

    #[inline]
    fn push_spaces(spaces: usize, buffer: &mut String) {
        for _ in 0..spaces {
            buffer.push(' ');
        }
    }

    fn process_blanks(
        _spaces: usize,
        buffer: &mut String,
        num: &str,
        ending: &str,
    ) -> Result<(), Error> {
        // Single blank line
        if num.is_empty() {
            buffer.push_str(ending);
        // Multiple blank lines
        } else {
            let num: syn::LitInt = syn::parse_str(num)?;
            let blanks: u32 = num.base10_parse()?;

            for _ in 0..blanks {
                buffer.push_str(ending);
            }
        }

        Ok(())
    }

    fn process_comments(
        spaces: usize,
        buffer: &mut String,
        s: &str,
        ending: &str,
    ) -> Result<(), Error> {
        // Single blank comment
        if s.is_empty() {
            Self::push_spaces(spaces, buffer);
            buffer.push_str(EMPTY_COMMENT);
            buffer.push_str(ending);
        // Multiple comments
        } else {
            let s: syn::LitStr = syn::parse_str(s)?;
            let comment = s.value();

            // Blank comment after parsing
            if comment.is_empty() {
                Self::push_spaces(spaces, buffer);
                buffer.push_str(EMPTY_COMMENT);
                buffer.push_str(ending);
            } else {
                for line in comment.lines() {
                    Self::push_spaces(spaces, buffer);

                    if line.is_empty() {
                        buffer.push_str(EMPTY_COMMENT);
                    } else {
                        buffer.push_str(COMMENT);
                        buffer.push_str(line);
                    }

                    buffer.push_str(ending);
                }
            }
        }

        Ok(())
    }

    // This is slightly different than comment in that we don't prepend a space but need to translate
    // the doc block literally (#[doc = "test"] == ///test <-- no prepended space)
    fn process_doc_block(
        spaces: usize,
        buffer: &mut String,
        s: &str,
        ending: &str,
    ) -> Result<(), Error> {
        // Single blank comment
        if s.is_empty() {
            Self::push_spaces(spaces, buffer);
            buffer.push_str(DOC_COMMENT);
            buffer.push_str(ending);
        // Multiple comments
        } else {
            let s: syn::LitStr = syn::parse_str(s)?;
            let comment = s.value();

            // Blank comment after parsing
            if comment.is_empty() {
                Self::push_spaces(spaces, buffer);
                buffer.push_str(DOC_COMMENT);
                buffer.push_str(ending);
            } else {
                for line in comment.lines() {
                    Self::push_spaces(spaces, buffer);
                    buffer.push_str(DOC_COMMENT);
                    buffer.push_str(line);
                    buffer.push_str(ending);
                }
            }
        }

        Ok(())
    }

    fn try_match_prefixes(
        &mut self,
        indent: usize,
        chars_matched: usize,
        prefixes: &[&[u8]],
        allow_ws_first: bool,
    ) -> Option<(usize, usize)> {
        // We already matched X chars before we got here (but didn't 'next()' after last match so minus 1)
        let mark_start_ident = self.curr_idx - ((chars_matched + indent) - 1);

        if self.try_ws_matches(prefixes, allow_ws_first) {
            let mark_start_value = self.curr_idx + 1;
            Some((mark_start_ident, mark_start_value))
        } else {
            None
        }
    }

    fn try_replace<F>(
        &mut self,
        spaces: usize,
        chars_matched: usize,
        suffixes: &[&[u8]],
        mark_start_ident: usize,
        mark_start_value: usize,
        f: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(usize, &mut String, &str, &str) -> Result<(), Error>,
    {
        // End of value (exclusive)
        let mark_end_value = self.curr_idx + (1 - chars_matched);

        if !self.try_ws_matches(suffixes, true) {
            return Err(Error::BadSourceCode(
                "Unable to match suffix on doc block or marker.".to_string(),
            ));
        }

        if let Some(ending) = self.detect_line_ending() {
            // Mark end of ident here (inclusive)
            let mark_end_ident = self.curr_idx + 1;

            // Copy everything up until this marker
            self.copy_to_marker(mark_start_ident, mark_end_ident);

            // Parse and output
            f(
                spaces,
                &mut self.buffer,
                &self.source[mark_start_value..mark_end_value],
                ending,
            )?;
            Ok(())
        } else {
            Err(Error::BadSourceCode("Expected CR or LF".to_string()))
        }
    }

    fn try_replace_blank_marker(&mut self, spaces: usize) -> Result<bool, Error> {
        // 6 or 7 sections to match: _blank_ ! ( [int] ) ; CRLF|LF

        match self.try_match_prefixes(spaces, 2, BLANK_START, false) {
            Some((ident_start, value_start)) => {
                self.skip_blank_param()?;

                self.try_replace(
                    spaces,
                    1,
                    BLANK_END,
                    ident_start,
                    value_start,
                    CopyingCursor::process_blanks,
                )?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn try_replace_comment_marker(&mut self, spaces: usize) -> Result<bool, Error> {
        // 6 or 7 sections to match: _comment_ ! ( [string] ) ; CRLF|LF

        match self.try_match_prefixes(spaces, 2, COMMENT_START, false) {
            Some((ident_start, value_start)) => {
                // Make sure it is empty or a string
                let (matched, suffix) = match self.try_skip_string()? {
                    // String
                    None => (0, COMMENT_END),
                    // Empty
                    Some(b')') => (1, COMMENT_END2),
                    Some(ch) => {
                        return Err(Error::BadSourceCode(format!(
                            "Expected ')' or string, but got: {}",
                            ch as char
                        )))
                    }
                };

                self.try_replace(
                    spaces,
                    matched,
                    suffix,
                    ident_start,
                    value_start,
                    CopyingCursor::process_comments,
                )?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn try_replace_doc_block(&mut self, spaces: usize) -> Result<bool, Error> {
        // 7 sections to match: # [ doc = <string> ] CRLF|LF

        match self.try_match_prefixes(spaces, 1, DOC_BLOCK_START, true) {
            Some((ident_start, value_start)) => {
                // Make sure it is a string
                match self.try_skip_string()? {
                    // String
                    None => {
                        self.try_replace(
                            spaces,
                            0,
                            DOC_BLOCK_END,
                            ident_start,
                            value_start,
                            CopyingCursor::process_doc_block,
                        )?;
                        Ok(true)
                    }
                    Some(ch) => Err(Error::BadSourceCode(format!(
                        "Expected string, but got: {}",
                        ch as char
                    ))),
                }
            }
            None => Ok(false),
        }
    }
}

pub(crate) fn replace_markers(s: &str, replace_doc_blocks: bool) -> Result<Cow<str>, Error> {
    match CopyingCursor::new(s) {
        Some(mut cursor) => {
            let mut indent = 0;

            loop {
                match cursor.curr {
                    // Possible raw string
                    b'r' => {
                        indent = 0;
                        if !cursor.try_skip_raw_string() {
                            continue;
                        }
                    }
                    // Regular string
                    b'\"' => {
                        indent = 0;
                        cursor.skip_string()
                    }
                    // Possible comment
                    b'/' => {
                        indent = 0;
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
                                if !cursor.try_replace_blank_marker(indent)? {
                                    indent = 0;
                                    continue;
                                }
                            }
                            // Possible comment marker
                            b'c' => {
                                if !cursor.try_replace_comment_marker(indent)? {
                                    indent = 0;
                                    continue;
                                }
                            }
                            // Nothing we are interested in
                            _ => {
                                indent = 0;
                                continue;
                            }
                        }

                        indent = 0;
                    }
                    // Possible doc block
                    b'#' if replace_doc_blocks => {
                        if !cursor.try_replace_doc_block(indent)? {
                            indent = 0;
                            continue;
                        }

                        indent = 0;
                    }
                    // Count spaces in front of our three special replacements
                    b' ' => {
                        indent += 1;
                    }
                    // Anything else
                    _ => {
                        indent = 0;
                    }
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

// *** Tests ***

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use pretty_assertions::assert_eq;

    use crate::replace::replace_markers;
    use crate::Error;

    #[test]
    fn blank() {
        let source = "";

        let actual = replace_markers(source, false).unwrap();
        let expected = source;

        assert_eq!(expected, actual);
        assert!(matches!(actual, Cow::Borrowed(_)));
    }

    #[test]
    fn no_replacements() {
        let source = r####"// _comment!_("comment");

/* /* nested comment */ */
        
/// This is a main function
fn main() {
    println!("hello world");
    println!(r##"hello raw world!"##);
}
_blank!_;
"####;

        let actual = replace_markers(source, false).unwrap();
        let expected = source;

        assert_eq!(expected, actual);
        assert!(matches!(actual, Cow::Borrowed(_)));
    }

    #[test]
    fn replace_comments() {
        let source = r####"// _comment!_("comment");

/* /* nested comment */ */
_comment_!("comment 1\n\ncomment 2");
_comment_!("test");
_comment!("skip this");
/// This is a main function
fn main() {
    println!(r##"hello raw world!"##);
    _comment_!(r"");
    _comment_!();
    println!("hello \nworld");
}

   _comment_ !
( r#"This is two
comments"# )
;
_blank!_;
"####;

        let actual = replace_markers(source, false).unwrap();
        let expected = r####"// _comment!_("comment");

/* /* nested comment */ */
// comment 1
//
// comment 2
// test
_comment!("skip this");
/// This is a main function
fn main() {
    println!(r##"hello raw world!"##);
    //
    //
    println!("hello \nworld");
}

   // This is two
   // comments
_blank!_;
"####;

        assert_eq!(expected, actual);
    }

    #[test]
    fn replace_blanks() {
        let source = r####"// _blank!_(5);

/* /* nested comment */ */
_blank_!(2);
_blank!_("skip this");
#[doc = "This is a main function"]
fn main() {
    let r#test = "hello";
    println!(r"hello raw world!");
    _blank_!();
    println!("hello \nworld");
}

      _blank_
!(
2
);
_blank!_;
"####;

        let actual = replace_markers(source, false).unwrap();
        let expected = r####"// _blank!_(5);

/* /* nested comment */ */


_blank!_("skip this");
#[doc = "This is a main function"]
fn main() {
    let r#test = "hello";
    println!(r"hello raw world!");

    println!("hello \nworld");
}



_blank!_;
"####;

        assert_eq!(expected, actual);
    }

    #[test]
    fn replace_doc_blocks() {
        let source = r####"// _blank!_(5);

/* not a nested comment */
#[doc = r#" This is a main function"#]
#[doc = r#" This is two doc
 comments"#]
#[cfg(feature = "main")]
#[doc(hidden)]
fn main() {
    println!(r##"hello raw world!"##);
    #[doc = ""]
    println!("hello \nworld");
}

#    [
doc
 = 
 " this is\n\n three doc comments"
 
 ]
fn test() {
}
_blank!_;
"####;

        let actual = replace_markers(source, true).unwrap();
        let expected = r####"// _blank!_(5);

/* not a nested comment */
/// This is a main function
/// This is two doc
/// comments
#[cfg(feature = "main")]
#[doc(hidden)]
fn main() {
    println!(r##"hello raw world!"##);
    ///
    println!("hello \nworld");
}

/// this is
///
/// three doc comments
fn test() {
}
_blank!_;
"####;

        assert_eq!(expected, actual);
    }

    #[test]
    fn replace_crlf() {
        let source = "_blank_!(2);\r\n";
        let actual = replace_markers(source, false).unwrap();

        let expected = "\r\n\r\n";
        assert_eq!(expected, actual);
    }

    #[test]
    fn marker_end_after_prefix() {
        assert!(matches!(
            replace_markers("_blank_!(", false),
            Err(Error::BadSourceCode(_))
        ));
    }

    #[test]
    fn marker_param_not_string() {
        assert!(matches!(
            replace_markers("_comment_!(blah);\n", false),
            Err(Error::BadSourceCode(_))
        ));
    }

    #[test]
    fn marker_bad_suffix() {
        assert!(matches!(
            replace_markers("_comment_!(\"blah\"];\n", false),
            Err(Error::BadSourceCode(_))
        ));
    }

    #[test]
    fn doc_block_string_not_closed() {
        assert!(matches!(
            replace_markers("#[doc = \"test]\n", true),
            Err(Error::BadSourceCode(_))
        ));
    }
}
