#![warn(missing_docs)]

//! Using the [doc_test] macro, we can take any [TokenStream](proc_macro2::TokenStream) and turn it into
//! a doctest [TokenStream](proc_macro2::TokenStream) that can be interpolated with another [quote](quote::quote)
//! macro invocation
//!
//! ```
//! use quote::quote;
//!
//! // Takes any `TokenStream` as input (but typically `quote` would be used)
//! let test = quote! {
//!     assert_eq!(fibonacci(10), 55);
//!     assert_eq!(fibonacci(1), 1);
//! };
//! let doc_test = quote_doctest::doc_test!(test).unwrap();
//!
//! // Interpolates into a regular `quote` invocation
//! let actual = quote! {
//!     /// This will run a compare between fib inputs and the outputs
//!     #doc_test
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
//!     #[doc = r" This will run a compare between fib inputs and the outputs"]
//!     #[doc = " ```"]
//!     #[doc = " assert_eq!(fibonacci(10), 55);"]
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

use std::env;
use std::error::Error;
use std::fmt;
use std::io::Write;
use std::process::{Command, Stdio};

use proc_macro2::TokenStream;
use quote::{quote, ToTokens};

const RUST_FMT: &str = "rustfmt";
const RUST_FMT_KEY: &str = "RUSTFMT";
const CODE_MARKER: &str = "/// ```\n";
const DOC_COMMENT: &str = "/// ";

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
            $crate::DocTestOptions::FormatAndGenMain($crate::Formatter::PrettyPlease, 4),
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
            $crate::DocTestOptions::FormatAndGenMain($crate::Formatter::RustFmt, 4),
        )
    };
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

/// Describes the kind of error that occurred while running the formatter
#[derive(Clone, Copy, Debug)]
pub enum FormatErrorKind {
    /// Unable to find or execute the `rustfmt` program
    UnableToExecRustFmt,
    /// Unable to write the source code to `rustfmt` stdin
    UnableToWriteStdin,
    /// Issues occurred during the execution of `rustfmt`
    ErrorDuringExec,
    /// The resulting stdout and stderror from `rustfmt` could not be converted to UTF8
    UTF8ConversionError,
    /// Formatter encountered an error in the source code it was trying to format. The display string
    /// contains the output from stderr (`rustfmt`) or error display value (`prettyplease`)
    BadSourceCode,
}

impl fmt::Display for FormatErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatErrorKind::UnableToExecRustFmt => f.write_str("Unable to execute 'rustfmt'"),
            FormatErrorKind::UnableToWriteStdin => {
                f.write_str("An error occurred while writing to 'rustfmt' stdin")
            }
            FormatErrorKind::ErrorDuringExec => {
                f.write_str("An error occurred while attempting to retrieve 'rustfmt' output")
            }
            FormatErrorKind::UTF8ConversionError => {
                f.write_str("Unable to convert 'rustfmt' output into UTF8")
            }
            FormatErrorKind::BadSourceCode => {
                f.write_str("Formatter was unable to parse the source code")
            }
        }
    }
}

/// Error type that is returned when [doc_test] fails while running the formatter
#[derive(Clone, Debug)]
pub struct FormatError {
    /// Describes the kind of error that occurred
    pub kind: FormatErrorKind,
    msg: String,
}

impl fmt::Display for FormatError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}: {}", self.kind, self.msg))
    }
}

impl Error for FormatError {}

impl FormatError {
    #[inline]
    fn new(kind: FormatErrorKind, msg: String) -> Self {
        Self { kind, msg }
    }
}

/// Attempts to translate this [TokenStream](proc_macro2::TokenStream) into a [String]. It takes an
/// optional [Formatter] which formats using either `prettyplease` or `rustfmt`. For `rustfmt`, It defaults
/// to calling whichever copy it finds via the system path, however, if the `RUSTFMT` environment variable
/// is set it will use that one. It returns a [String] of the formatted code (or a single line of
/// unformatted text, if `fmt` is `None`) or a [FormatError] error, if one occurred.
pub fn tokens_to_string(
    tokens: TokenStream,
    fmt: Option<Formatter>,
) -> Result<String, FormatError> {
    match fmt {
        #[cfg(feature = "prettyplease")]
        Some(Formatter::PrettyPlease) => prettyplz(tokens),
        Some(Formatter::RustFmt) => {
            let (src, _) = rustfmt_str(tokens)?;
            Ok(src)
        }
        None => Ok(tokens.to_string()),
    }
}

#[doc(hidden)]
pub fn make_doc_test(
    mut tokens: TokenStream,
    options: DocTestOptions,
) -> Result<TokenStream, FormatError> {
    let (fmt, gen_main, strip_indent) = options.options();

    // Surround with main, if needed (we can't remove it unless we are formatting)
    if gen_main {
        tokens = quote! {
            fn main() { #tokens }
        };
    }

    // Format, if required, and the break into lines
    let src = tokens_to_string(tokens, fmt)?;
    let lines = to_source_lines(&src, gen_main);

    // Assemble the lines back into a string
    let prefix = " ".repeat(strip_indent);
    let doc_test = assemble_doc_test(lines, src.len(), prefix);

    // Parse the new string into document attributes and return a token stream
    // Safety - these should always succeed since we created them
    let docs: DocAttrs = syn::parse_str(&doc_test).expect("bad doc attributes");
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

fn assemble_doc_test(lines: Vec<&str>, cap: usize, prefix: String) -> String {
    // Unlikely to be this big, but better than reallocating
    let mut doc_test = String::with_capacity(cap * 2);

    // Build code from lines
    doc_test.push_str(CODE_MARKER);
    for mut line in lines {
        // Strip whitespace left over from main
        line = line.strip_prefix(&prefix).unwrap_or(line);

        doc_test.push_str(DOC_COMMENT);
        doc_test.push_str(line);
        doc_test.push('\n');
    }
    doc_test.push_str(CODE_MARKER);

    doc_test
}

#[cfg(feature = "prettyplease")]
fn prettyplz(tokens: TokenStream) -> Result<String, FormatError> {
    let f = syn::parse2::<syn::File>(tokens)
        .map_err(|err| FormatError::new(FormatErrorKind::BadSourceCode, err.to_string()))?;
    Ok(prettyplease::unparse(&f))
}

fn rustfmt_str(tokens: TokenStream) -> Result<(String, String), FormatError> {
    let s = tokens.to_string();

    // Use 'rustfmt' specified by the environment var, if specified, else use the default
    let rustfmt = env::var(RUST_FMT_KEY).unwrap_or_else(|_| RUST_FMT.to_string());

    // Launch rustfmt
    let mut proc = Command::new(&rustfmt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| FormatError::new(FormatErrorKind::UnableToExecRustFmt, err.to_string()))?;

    // Get stdin and send our source code to it to be formatted
    // Safety: Can't panic - we captured stdin above
    let mut stdin = proc.stdin.take().unwrap();
    stdin
        .write_all(s.as_bytes())
        .map_err(|err| FormatError::new(FormatErrorKind::UnableToWriteStdin, err.to_string()))?;
    // Close stdin
    drop(stdin);

    // Parse the results and return stdout/stderr
    let output = proc
        .wait_with_output()
        .map_err(|err| FormatError::new(FormatErrorKind::ErrorDuringExec, err.to_string()))?;
    let stderr = String::from_utf8(output.stderr)
        .map_err(|err| FormatError::new(FormatErrorKind::UTF8ConversionError, err.to_string()))?;

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout).map_err(|err| {
            FormatError::new(FormatErrorKind::UTF8ConversionError, err.to_string())
        })?;
        Ok((stdout, stderr))
    } else {
        Err(FormatError::new(FormatErrorKind::BadSourceCode, stderr))
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use crate::{
        tokens_to_string, DocTestOptions, FormatError, FormatErrorKind, Formatter, RUST_FMT,
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

        assert_eq!(actual.to_string(), expected.to_string());
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

        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn bad_path_rustfmt() {
        temp_env::with_var(
            RUST_FMT_KEY,
            Some("this_is_never_going_to_be_a_valid_executable"),
            || match tokens_to_string(quote! {}, Some(Formatter::RustFmt)) {
                Err(FormatError {
                    kind: FormatErrorKind::UnableToExecRustFmt,
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
            Err(FormatError {
                kind: FormatErrorKind::BadSourceCode,
                ..
            }) => {}
            _ => panic!("'rustfmt' should have failed due to bad source code"),
        }
    }
}
