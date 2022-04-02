#![warn(missing_docs)]

//! Using the [doc_test] macro, we can take any [TokenStream](proc_macro2::TokenStream) and turn it into
//! a doctest [TokenStream](proc_macro2::TokenStream) that can be interpolated in any [quote](quote::quote)
//! macro invocation. The [doc_comment] function takes any string and turns it into a [TokenStream](proc_macro2::TokenStream).
//!
//! ```
//! use quote::quote;
//! use quote_doctest::{doc_comment, doc_test};
//!
//! // Takes any `TokenStream` as input (but typically `quote` would be used)
//! let test = doc_test!(quote! {
//!     _comment!("Calling fibonacci with 10 returns 55");
//!     assert_eq!(fibonacci(10), 55);
//!
//!     _blank!();
//!     _comment!("Calling fibonacci with 1 simply returns 1");
//!     assert_eq!(fibonacci(1), 1);
//! }).unwrap();
//!
//! let comment = doc_comment("This compares between fib inputs and outputs\n\n").unwrap();
//!
//! // Interpolates into a regular `quote` invocation
//! let actual = quote! {
//!     #comment
//!     #test
//!     fn fibonacci(n: u64) -> u64 {
//!         match n {
//!             0 => 1,
//!             1 => 1,
//!             n => fibonacci(n - 1) + fibonacci(n - 2),
//!         }
//!     }
//! };
//!
//! // This is what is generated:
//! let expected = quote! {
//!     #[doc = " This compares between fib inputs and outputs"]
//!     #[doc = ""]
//!     #[doc = " ```"]
//!     #[doc = " // Calling fibonacci with 10 returns 55"]
//!     #[doc = " assert_eq!(fibonacci(10), 55);"]
//!     #[doc = ""]
//!     #[doc = " // Calling fibonacci with 1 simply returns 1"]
//!     #[doc = " assert_eq!(fibonacci(1), 1);"]
//!     #[doc = " ```"]
//!     fn fibonacci(n: u64) -> u64 {
//!         match n {
//!             0 => 1,
//!             1 => 1,
//!             n => fibonacci(n - 1) + fibonacci(n - 2),
//!         }
//!     }
//! };
//!
//! assert_eq!(expected.to_string(), actual.to_string());
//! ```

// Trick to test README samples (from: https://github.com/rust-lang/cargo/issues/383#issuecomment-720873790)
#[cfg(doctest)]
mod test_readme {
    macro_rules! external_doc_test {
        ($x:expr) => {
            #[doc = $x]
            extern "C" {}
        };
    }

    external_doc_test!(include_str!("../README.md"));
}

use std::io::Write;
use std::process::{Command, Stdio};
use std::{cmp, env, error, fmt};

use proc_macro2::TokenStream;
use quote::{quote, ToTokens};

const RUST_FMT: &str = "rustfmt";
const RUST_FMT_KEY: &str = "RUSTFMT";
const CODE_MARKER: &str = "/// ```\n";
const DOC_COMMENT: &str = "/// ";
const EMPTY_DOC_COMMENT: &str = "///";
const COMMENT: &str = "// ";
const EMPTY_COMMENT: &str = "//";

const BLANK_IDENT: &str = "_blank";
const COMMENT_IDENT: &str = "_comment";

const MIN_BUFF_SIZE: usize = 128;

/// The default amount of formatter indent to remove (when generating `main`)
pub const FORMATTER_INDENT: usize = 4;

/// Creates a doctest from a [TokenStream](proc_macro2::TokenStream). Typically that is all that
/// is supplied, however, there is an optional parameter of type [DocTestOptions] that can be supplied
/// to fine tune whether or not formating is used, choose a formatter (either `pretty_please` or `rustfmt`),
/// or whether a main function generated (it is required if formatting, but one can be specified manually).
///
/// If formatting and using `rustfmt`, by default it will attempt to be located in the system path.
/// If the `RUSTFMT` environment variable is set, however, that will be used instead.
#[macro_export]
macro_rules! doc_test {
    ($tokens:expr) => {
        $crate::__default_doc_test!($tokens)
    };
    ($tokens:expr, $options:expr) => {
        $crate::make_doc_test($tokens, $options)
    };
}

#[cfg(feature = "prettyplease")]
#[doc(hidden)]
#[macro_export]
macro_rules! __default_doc_test {
    ($tokens:expr) => {
        $crate::make_doc_test(
            $tokens,
            $crate::DocTestOptions::FormatAndGenMain(
                $crate::Formatter::PrettyPlease,
                $crate::FORMATTER_INDENT,
            ),
        )
    };
}

#[cfg(not(feature = "prettyplease"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __default_doc_test {
    ($tokens:expr) => {
        $crate::make_doc_test(
            $tokens,
            $crate::DocTestOptions::FormatAndGenMain(
                $crate::Formatter::RustFmt,
                $crate::FORMATTER_INDENT,
            ),
        )
    };
}

/// A "marker" macro used to mark locations in the [TokenStream](proc_macro2::TokenStream) where blank
/// lines should be inserted. If no parameter is given, one blank line is assumed, otherwise the integer
/// literal specified gives the # of blank lines to insert.
///
/// Since these "marker" macros aren't actually executed (or used), they must be called in short form. Using
/// the fully qualified name or a renamed import will not work. To avoid naming collisions, this macro
/// has been prefixed with an underscore.
///
/// Actually executing this macro has no effect.
#[macro_export]
macro_rules! _blank {
    () => {};
    ($lit:literal) => {};
}

/// A "marker" macro used to mark locations in the [TokenStream](proc_macro2::TokenStream) where comments
/// should be inserted. If no parameter is given, a single blank is assumed, otherwise the string literal
/// specified is broken into lines and those comments will be inserted individually.
///
/// Since these "marker" macros aren't actually executed (or used), they must be called in short form. Using
/// the fully qualified name or a renamed import will not work. To avoid naming collisions, this macro
/// has been prefixed with an underscore.
///
/// Actually executing this macro has no effect.
#[macro_export]
macro_rules! _comment {
    () => {};
    ($lit:literal) => {};
}

// Requires 'extra-traits' feature on syn - can just enable if ever needed for debugging
// #[derive(Debug)]
struct DocAttrs {
    docs: Vec<syn::Attribute>,
}

impl syn::parse::Parse for DocAttrs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let docs = input.call(syn::Attribute::parse_outer)?;
        Ok(Self { docs })
    }
}

impl ToTokens for DocAttrs {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        for doc in &self.docs {
            doc.to_tokens(tokens);
        }
    }
}

/// The formatter used to format source code - either `prettyplease` or the system `rustfmt`
#[derive(Clone, Copy, Debug)]
pub enum Formatter {
    /// Format using `prettyplease` crate
    #[cfg(feature = "prettyplease")]
    PrettyPlease,
    /// Format by calling out to the system `rustfmt`
    RustFmt,
}

/// Optional enum passed to [doc_test] for different configuration options
#[derive(Clone, Copy, Debug)]
pub enum DocTestOptions {
    /// TokenStream is not formatted and no main function is generated. The doctest will be a single line
    NoFormatOrGenMain,
    /// TokenStream is formatted only by the specified formatter. The source code must be inside a
    /// function or it will cause an error
    FormatOnly(Formatter),
    /// TokenStream is formatted by the specified formatter and a main function is generated that is
    /// later stripped after formatting. The `usize` parameter is the number of indent spaces to be
    /// stripped (typically this number should be 4)
    FormatAndGenMain(Formatter, usize),
}

impl DocTestOptions {
    #[inline]
    fn options(self) -> (Option<Formatter>, bool, usize) {
        match self {
            DocTestOptions::NoFormatOrGenMain => (None, false, 0),
            DocTestOptions::FormatOnly(fmt) => (Some(fmt), false, 0),
            DocTestOptions::FormatAndGenMain(fmt, strip_indent) => (Some(fmt), true, strip_indent),
        }
    }
}

/// Describes the kind of error that occurred
#[derive(Clone, Copy, Debug)]
pub enum ErrorKind {
    /// Unable to find or execute the `rustfmt` program
    UnableToExecRustFmt,
    /// Unable to write the source code to `rustfmt` stdin
    UnableToWriteStdin,
    /// Issues occurred during the execution of `rustfmt`
    ErrorDuringExec,
    /// The resulting stdout and stderror from `rustfmt` could not be converted to UTF8
    UTF8ConversionError,
    /// An error was encountered parsing the source code. This is often, but not always, reported
    /// by the formatter. The display string/ contains the output from stderr (`rustfmt`) or error
    /// display value (`prettyplease` and internal parsiing)
    BadSourceCode,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::UnableToExecRustFmt => f.write_str("Unable to execute 'rustfmt'"),
            ErrorKind::UnableToWriteStdin => {
                f.write_str("An error occurred while writing to 'rustfmt' stdin")
            }
            ErrorKind::ErrorDuringExec => {
                f.write_str("An error occurred while attempting to retrieve 'rustfmt' output")
            }
            ErrorKind::UTF8ConversionError => {
                f.write_str("Unable to convert 'rustfmt' output into UTF8")
            }
            ErrorKind::BadSourceCode => {
                f.write_str("Syntax error - unable to parse the source code")
            }
        }
    }
}

/// Error type that is returned when a macro or function fails
#[derive(Clone, Debug)]
pub struct Error {
    /// Describes the kind of error that occurred
    pub kind: ErrorKind,
    msg: String,
}

impl fmt::Display for Error {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}: {}", self.kind, self.msg))
    }
}

impl error::Error for Error {}

impl Error {
    #[inline]
    fn new(kind: ErrorKind, msg: String) -> Self {
        Self { kind, msg }
    }
}

/// Attempts to translate this [TokenStream](proc_macro2::TokenStream) into a [String]. It takes an
/// optional [Formatter] which formats using either `prettyplease` or `rustfmt`. For `rustfmt`, It defaults
/// to calling whichever copy it finds via the system path, however, if the `RUSTFMT` environment variable
/// is set it will use that one. It returns a [String] of the formatted code (or a single line of
/// unformatted text, if `fmt` is `None`) or an [Error] error, if one occurred.
pub fn tokens_to_string(tokens: TokenStream, fmt: Option<Formatter>) -> Result<String, Error> {
    match fmt {
        #[cfg(feature = "prettyplease")]
        Some(Formatter::PrettyPlease) => prettyplz(tokens),
        Some(Formatter::RustFmt) => {
            let (src, _) = rustfmt(tokens)?;
            Ok(src)
        }
        None => Ok(tokens.to_string()),
    }
}

/// Creates a doc comment for interpolation into a [TokenStream](proc_macro2::TokenStream). It takes
/// a string as input, splits it by line, and inserts one doc comment per line.
///
/// The value of this function over simply using `///` is that `quote` does not currently interpolate.
/// It will for `#[doc]` but only for one comment at a time. This function allows insertion of any
/// number of lines with one comment per line.
///
/// ```
/// use quote::quote;
/// use quote_doctest::doc_comment;
///
/// let actual = doc_comment("this\nwill be\n\nmultiple comments\n\n").unwrap();
/// let expected = quote! {
///     #[doc = " this"]
///     #[doc = " will be"]
///     #[doc = ""]
///     #[doc = " multiple comments"]
///     #[doc = ""]
/// };
///
/// assert_eq!(expected.to_string(), actual.to_string());
/// ```
pub fn doc_comment(comment: impl AsRef<str>) -> Result<TokenStream, Error> {
    let comment = comment.as_ref();

    // Go big - no sense reallocating unnecessarily
    let mut buffer = String::with_capacity(cmp::max(comment.len() * 2, MIN_BUFF_SIZE));

    for comm in comment.lines() {
        if comm.is_empty() {
            buffer.push_str(EMPTY_DOC_COMMENT);
        } else {
            buffer.push_str(DOC_COMMENT);
            buffer.push_str(comm);
        }

        buffer.push('\n');
    }

    // Parse into doc attrs and then return final token stream
    let docs: DocAttrs = syn::parse_str(&buffer)
        .map_err(|err| Error::new(ErrorKind::BadSourceCode, err.to_string()))?;
    Ok(docs.to_token_stream())
}

#[doc(hidden)]
pub fn make_doc_test(
    mut tokens: TokenStream,
    options: DocTestOptions,
) -> Result<TokenStream, Error> {
    let (fmt, gen_main, strip_indent) = options.options();

    // Surround with main, if needed (we can't remove it unless we are formatting)
    if gen_main {
        tokens = quote! {
            fn main() { #tokens }
        };
    }

    // Format, if required, and then break into lines
    let src = tokens_to_string(tokens, fmt)?;
    let lines = to_source_lines(&src, gen_main);

    // Assemble the lines back into a string while transforming the data
    let prefix = " ".repeat(strip_indent);
    let doc_test = assemble_doc_test(lines, src.len(), prefix)?;

    // Parse the new string into document attributes and return a token stream
    let docs: DocAttrs = syn::parse_str(&doc_test)
        .map_err(|err| Error::new(ErrorKind::BadSourceCode, err.to_string()))?;
    Ok(docs.to_token_stream())
}

fn to_source_lines(src: &str, gen_main: bool) -> Vec<&str> {
    // Split string source code into lines
    let lines = src.lines();

    // Remove `fn main () {`, if we added it
    if gen_main {
        // Skip 'fn main {'
        let mut lines = lines.skip(1).collect::<Vec<_>>();
        // Remove the trailing `}`
        lines.pop();
        lines
    } else {
        lines.collect()
    }
}

fn assemble_doc_test(lines: Vec<&str>, cap: usize, prefix: String) -> Result<String, Error> {
    // Unlikely to be this big, but better than reallocating
    let mut buffer = String::with_capacity(cmp::max(cap * 2, MIN_BUFF_SIZE));

    // Build code from lines
    buffer.push_str(CODE_MARKER);
    for mut line in lines {
        // Strip whitespace left over from main
        line = line.strip_prefix(&prefix).unwrap_or(line);
        process_line(line, &mut buffer)?;
    }
    buffer.push_str(CODE_MARKER);

    Ok(buffer)
}

fn process_line(line: &str, buffer: &mut String) -> Result<(), Error> {
    if !line.is_empty() {
        // First, see if this is one of our special macros (it won't parse unless we strip semicolon)
        if let Ok(m) = syn::parse_str::<syn::Macro>(&line[..line.len() - 1]) {
            // Our comment macro
            if m.path.is_ident(COMMENT_IDENT) {
                // Blank comment
                if m.tokens.is_empty() {
                    // Same as a blank line really
                    buffer.push_str(EMPTY_DOC_COMMENT);
                    buffer.push('\n');

                    return Ok(());
                // Actual comments present
                } else {
                    let comment = m
                        .parse_body::<syn::LitStr>()
                        .map_err(|err| Error::new(ErrorKind::BadSourceCode, err.to_string()))?
                        .value();

                    // Insert one comment per line
                    for comm in comment.lines() {
                        buffer.push_str(DOC_COMMENT);

                        if !comm.is_empty() {
                            buffer.push_str(COMMENT);
                            buffer.push_str(comm);
                        } else {
                            buffer.push_str(EMPTY_COMMENT);
                        }

                        buffer.push('\n');
                    }

                    return Ok(());
                }
            // Our blank macro
            } else if m.path.is_ident(BLANK_IDENT) {
                // 1 line
                let num_lines = if m.tokens.is_empty() {
                    1u32
                // Multiple lines (or at least number of specified)
                } else {
                    let num_lines = m
                        .parse_body::<syn::LitInt>()
                        .map_err(|err| Error::new(ErrorKind::BadSourceCode, err.to_string()))?;
                    num_lines
                        .base10_parse()
                        .map_err(|err| Error::new(ErrorKind::BadSourceCode, err.to_string()))?
                };

                // Insert correct # of blank lines
                for _ in 0..num_lines {
                    buffer.push_str(EMPTY_DOC_COMMENT);
                    buffer.push('\n');
                }

                return Ok(());
            }
        }
    }

    // Allow it to drop through to here from any level of the if above if not matched

    // Regular line processing
    buffer.push_str(DOC_COMMENT);
    buffer.push_str(line);
    buffer.push('\n');
    Ok(())
}

#[cfg(feature = "prettyplease")]
fn prettyplz(tokens: TokenStream) -> Result<String, Error> {
    let f = syn::parse2::<syn::File>(tokens)
        .map_err(|err| Error::new(ErrorKind::BadSourceCode, err.to_string()))?;
    Ok(prettyplease::unparse(&f))
}

fn rustfmt(tokens: TokenStream) -> Result<(String, String), Error> {
    let s = tokens.to_string();

    // Use 'rustfmt' specified by the environment var, if specified, else use the default
    let rustfmt = env::var(RUST_FMT_KEY).unwrap_or_else(|_| RUST_FMT.to_string());

    // Launch rustfmt
    let mut proc = Command::new(&rustfmt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| Error::new(ErrorKind::UnableToExecRustFmt, err.to_string()))?;

    // Get stdin and send our source code to it to be formatted
    // Safety: Can't panic - we captured stdin above
    let mut stdin = proc.stdin.take().unwrap();
    stdin
        .write_all(s.as_bytes())
        .map_err(|err| Error::new(ErrorKind::UnableToWriteStdin, err.to_string()))?;
    // Close stdin
    drop(stdin);

    // Parse the results and return stdout/stderr
    let output = proc
        .wait_with_output()
        .map_err(|err| Error::new(ErrorKind::ErrorDuringExec, err.to_string()))?;
    let stderr = String::from_utf8(output.stderr)
        .map_err(|err| Error::new(ErrorKind::UTF8ConversionError, err.to_string()))?;

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout)
            .map_err(|err| Error::new(ErrorKind::UTF8ConversionError, err.to_string()))?;
        Ok((stdout, stderr))
    } else {
        Err(Error::new(ErrorKind::BadSourceCode, stderr))
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use crate::{
        tokens_to_string, DocTestOptions, Error, ErrorKind, Formatter, FORMATTER_INDENT, RUST_FMT,
        RUST_FMT_KEY,
    };

    #[test]
    fn rustfmt_format_only() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            format_only(Formatter::RustFmt);
        });
    }

    #[cfg(feature = "prettyplease")]
    #[test]
    fn prettyplz_format_only() {
        format_only(Formatter::PrettyPlease);
    }

    fn format_only(fmt: Formatter) {
        let code = quote! {
            fn main() {
                assert_eq!(fibonacci(10), 55);
                assert_eq!(fibonacci(1), 1);
            }
        };

        let actual = doc_test!(code, DocTestOptions::FormatOnly(fmt)).unwrap();

        let expected = quote! {
            #[doc = " ```"]
            #[doc = " fn main() {"]
            #[doc = "     assert_eq!(fibonacci(10), 55);"]
            #[doc = "     assert_eq!(fibonacci(1), 1);"]
            #[doc = " }"]
            #[doc = " ```"]
        };

        assert_eq!(expected.to_string(), actual.to_string());
    }

    #[test]
    fn no_format_or_gen_main() {
        let code = quote! {
            fn main() {
                assert_eq!(fibonacci(10), 55);
                assert_eq!(fibonacci(1), 1);
            }
        };

        let actual = doc_test!(code, DocTestOptions::NoFormatOrGenMain).unwrap();
        let expected = quote! {
            #[doc = " ```"]
            #[doc = " fn main () { assert_eq ! (fibonacci (10) , 55) ; assert_eq ! (fibonacci (1) , 1) ; }"]
            #[doc = " ```"]
        };

        assert_eq!(expected.to_string(), actual.to_string());
    }

    #[test]
    fn bad_path_rustfmt() {
        temp_env::with_var(
            RUST_FMT_KEY,
            Some("this_is_never_going_to_be_a_valid_executable"),
            || match tokens_to_string(quote! {}, Some(Formatter::RustFmt)) {
                Err(Error {
                    kind: ErrorKind::UnableToExecRustFmt,
                    ..
                }) => {}
                _ => panic!("'rustfmt' should have failed due to bad path"),
            },
        );
    }

    #[test]
    fn rustfmt_bad_source_code() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            bad_source_code(Formatter::RustFmt);
        });
    }

    #[cfg(feature = "prettyplease")]
    #[test]
    fn prettyplz_bad_source_code() {
        bad_source_code(Formatter::PrettyPlease);
    }

    fn bad_source_code(fmt: Formatter) {
        match tokens_to_string(quote! {"blah blah blah"}, Some(fmt)) {
            Err(Error {
                kind: ErrorKind::BadSourceCode,
                ..
            }) => {}
            _ => panic!("'rustfmt' should have failed due to bad source code"),
        }
    }

    #[test]
    fn rustfmt_comment_marker() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            comment_marker(Formatter::RustFmt);
        });
    }

    #[cfg(feature = "prettyplease")]
    #[test]
    fn prettyplz_comment_marker() {
        comment_marker(Formatter::PrettyPlease);
    }

    fn comment_marker(fmt: Formatter) {
        let code = quote! {
            assert_eq!(fibonacci(10), 55);

            // Should translate to a blank line
            _comment!();
            _comment!("first line\n\nsecond line");
            assert_eq!(fibonacci(1), 1);
        };

        let actual = doc_test!(
            code,
            DocTestOptions::FormatAndGenMain(fmt, FORMATTER_INDENT)
        )
        .unwrap();

        let expected = quote! {
            #[doc = " ```"]
            #[doc = " assert_eq!(fibonacci(10), 55);"]
            #[doc = ""]
            #[doc = " // first line"]
            #[doc = " //"]
            #[doc = " // second line"]
            #[doc = " assert_eq!(fibonacci(1), 1);"]
            #[doc = " ```"]
        };

        assert_eq!(expected.to_string(), actual.to_string());
    }

    #[test]
    fn rustfmt_blank_marker() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            blank_marker(Formatter::RustFmt);
        });
    }

    #[cfg(feature = "prettyplease")]
    #[test]
    fn prettyplz_blank_marker() {
        blank_marker(Formatter::PrettyPlease);
    }

    fn blank_marker(fmt: Formatter) {
        let code = quote! {
            assert_eq!(fibonacci(10), 55);

            // Should translate to a single blank line
            _blank!();
            assert_eq!(fibonacci(1), 1);

            // Should translate to multiple blank lines
            _blank!(2);
        };

        let actual = doc_test!(
            code,
            DocTestOptions::FormatAndGenMain(fmt, FORMATTER_INDENT)
        )
        .unwrap();

        let expected = quote! {
            #[doc = " ```"]
            #[doc = " assert_eq!(fibonacci(10), 55);"]
            #[doc = ""]
            #[doc = " assert_eq!(fibonacci(1), 1);"]
            #[doc = ""]
            #[doc = ""]
            #[doc = " ```"]
        };

        assert_eq!(expected.to_string(), actual.to_string());
    }
}
