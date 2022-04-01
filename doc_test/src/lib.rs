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
//!     #[doc = "```"]
//!     #[doc = "assert_eq!(fibonacci(10), 55);"]
//!     #[doc = "assert_eq!(fibonacci(1), 1);"]
//!     #[doc = "```"]
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
use std::fmt::{Display, Formatter};
use std::io::Write;
use std::process::{Command, Stdio};

use proc_macro2::TokenStream;
use quote::quote;

const RUST_FMT: &str = "rustfmt";
const RUST_FMT_KEY: &str = "RUSTFMT";
const CODE_MARKER: &str = "```";

/// Creates a doctest from a [TokenStream](proc_macro2::TokenStream). Typically that is all that
/// is supplied, however, there is an optional parameter of type [DocTestOptions] that can be supplied
/// to fine tune whether or not rustfmt is used or a main function generated.
///
/// If formatting is done, by default `rustfmt` will attempt to be located in the system path. If the
/// `RUSTFMT` environment variable is set, however, that will be used instead.
#[macro_export]
macro_rules! doc_test {
    ($tokens:expr) => {
        $crate::make_doc_test($tokens, $crate::DocTestOptions::FormatAndGenMain(4))
    };
    ($tokens:expr, $options:expr) => {
        $crate::make_doc_test($tokens, $options)
    };
}

/// Optional enum passed to [doc_test] for different configuration options
#[derive(Clone, Copy, Debug)]
pub enum DocTestOptions {
    /// TokenStream is not sent to 'rustfmt' and no main function is generated. The doctest will be a single line
    NoFormatOrGenMain,
    /// TokenStream is sent to 'rustfmt' only. The source code must be inside a function or it will cause an error
    FormatOnly,
    /// TokenStream is formatted and a main function is generated that is later stripped after formatting.
    /// The `usize` parameter is the number of indent spaces to be stripped (typically this number should be 4)
    FormatAndGenMain(usize),
}

impl DocTestOptions {
    #[inline]
    fn config(&self) -> (bool, bool, usize) {
        match self {
            DocTestOptions::NoFormatOrGenMain => (false, false, 0),
            DocTestOptions::FormatOnly => (true, false, 0),
            DocTestOptions::FormatAndGenMain(_) => (true, true, 4),
        }
    }
}

/// Describes the kind of error that occurred while running 'rustfmt'
#[derive(Clone, Copy, Debug)]
pub enum RustFmtErrorKind {
    /// Unable to find or execute the `rustfmt` program
    UnableToExecRustFmt,
    /// Unable to write the source code to stdin
    UnableToWriteStdin,
    /// Issues occurred during the execution of 'rustfmt'
    ErrorDuringExec,
    /// The resulting stdout and stderror from 'rustfmt' could not be converted to UTF8
    UTF8ConversionError,
    /// 'rustfmt' encountered an error in the source code it was trying to format. The display string
    /// contains the output from stderr
    BadSourceCode,
}

impl Display for RustFmtErrorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RustFmtErrorKind::UnableToExecRustFmt => f.write_str("Unable to execute 'rustfmt'"),
            RustFmtErrorKind::UnableToWriteStdin => {
                f.write_str("An error occurred while writing to stdin")
            }
            RustFmtErrorKind::ErrorDuringExec => {
                f.write_str("An error occurred while attempting to retrieve rustfmt output")
            }
            RustFmtErrorKind::UTF8ConversionError => {
                f.write_str("Unable to convert rustfmt output into UTF8")
            }
            RustFmtErrorKind::BadSourceCode => {
                f.write_str("Rustfmt was unable to parse the source code")
            }
        }
    }
}

/// Error type that is returned when 'doc_test' fails while running 'rustfmt'
#[derive(Clone, Debug)]
pub struct RustFmtError {
    /// Describes the kind of error that occurred
    pub kind: RustFmtErrorKind,
    msg: String,
}

impl Display for RustFmtError {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}: {}", self.kind, self.msg))
    }
}

impl Error for RustFmtError {}

impl RustFmtError {
    #[inline]
    fn new(kind: RustFmtErrorKind, msg: String) -> Self {
        Self { kind, msg }
    }
}

#[doc(hidden)]
pub fn make_doc_test(
    mut tokens: TokenStream,
    options: DocTestOptions,
) -> Result<TokenStream, RustFmtError> {
    let (fmt, gen_main, strip_indent) = options.config();

    // Surround with main, if needed (and we can't remove it unless we are formatting)
    if gen_main {
        tokens = quote! {
            fn main() { #tokens }
        };
    }

    // Convert to string source code format
    let mut src = tokens.to_string();

    // Format it, if requested
    if fmt {
        let (source, _) = format_rs_str(&src)?;
        src = source;
    }

    // Split string source code into lines
    let lines = src.lines();
    let cap = lines.size_hint().0 + 2;

    // Remove `fn main () {`, if we added it
    let lines = if gen_main {
        lines.skip(1)
    } else {
        lines.skip(0)
    };
    let mut doc_test = Vec::with_capacity(cap);
    let prefix = " ".repeat(strip_indent);

    doc_test.push(CODE_MARKER);
    for mut line in lines {
        // Strip whitespace left over from main
        line = line.strip_prefix(&prefix).unwrap_or(line);
        doc_test.push(line);
    }
    // Remove the trailing `}`
    if gen_main {
        doc_test.pop();
    }
    doc_test.push(CODE_MARKER);

    // Convert into doc attributes
    Ok(quote! {
        #( #[doc = #doc_test] )*
    })
}

/// Formats a string of Rust source code via rustfmt. It defaults to calling whichever copy it finds
/// via the system path, however, if the `RUSTFMT` environment variable is set it will use that one.
/// It returns a tuple of stdout output and stderr output respectively, or an error, if one occurred.
pub fn format_rs_str(s: impl AsRef<str>) -> Result<(String, String), RustFmtError> {
    // Use 'rustfmt' specified by the environment var, if specified, else use the default
    let rustfmt = env::var(RUST_FMT_KEY).unwrap_or_else(|_| RUST_FMT.to_string());

    // Launch rustfmt
    let mut proc = Command::new(&rustfmt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| RustFmtError::new(RustFmtErrorKind::UnableToExecRustFmt, err.to_string()))?;

    // Get stdin and send our source code to it to be formatted
    // Safety: Can't panic - we captured stdin above
    let mut stdin = proc.stdin.take().unwrap();
    stdin
        .write_all(s.as_ref().as_bytes())
        .map_err(|err| RustFmtError::new(RustFmtErrorKind::UnableToWriteStdin, err.to_string()))?;
    // Close stdin
    drop(stdin);

    // Parse the results and return stdout/stderr
    let output = proc
        .wait_with_output()
        .map_err(|err| RustFmtError::new(RustFmtErrorKind::ErrorDuringExec, err.to_string()))?;
    let stderr = String::from_utf8(output.stderr)
        .map_err(|err| RustFmtError::new(RustFmtErrorKind::UTF8ConversionError, err.to_string()))?;

    if output.status.success() {
        let stdout = String::from_utf8(output.stdout).map_err(|err| {
            RustFmtError::new(RustFmtErrorKind::UTF8ConversionError, err.to_string())
        })?;
        Ok((stdout, stderr))
    } else {
        Err(RustFmtError::new(RustFmtErrorKind::BadSourceCode, stderr))
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use crate::{
        format_rs_str, DocTestOptions, RustFmtError, RustFmtErrorKind, RUST_FMT, RUST_FMT_KEY,
    };

    #[test]
    fn format_only() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            let code = quote! {
                fn main() {
                    assert_eq!(fibonacci(10), 55);
                    assert_eq!(fibonacci(1), 1);
                }
            };

            let actual = doc_test!(code, DocTestOptions::FormatOnly).unwrap();

            let expected = quote! {
                #[doc = "```"]
                #[doc = "fn main() {"]
                #[doc = "    assert_eq!(fibonacci(10), 55);"]
                #[doc = "    assert_eq!(fibonacci(1), 1);"]
                #[doc = "}"]
                #[doc = "```"]
            };

            assert_eq!(actual.to_string(), expected.to_string());
        });
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
            #[doc = "```"]
            #[doc = "fn main () { assert_eq ! (fibonacci (10) , 55) ; assert_eq ! (fibonacci (1) , 1) ; }"]
            #[doc = "```"]
        };

        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn bad_rustfmt() {
        temp_env::with_var(
            RUST_FMT_KEY,
            Some("this_is_never_going_to_be_a_valid_executable"),
            || match format_rs_str("") {
                Err(RustFmtError {
                    kind: RustFmtErrorKind::UnableToExecRustFmt,
                    ..
                }) => {}
                _ => panic!("'rustfmt' should have failed due to bad path"),
            },
        );
    }

    #[test]
    fn bad_source_code() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            match format_rs_str("blah blah blah") {
                Err(RustFmtError {
                    kind: RustFmtErrorKind::BadSourceCode,
                    ..
                }) => {}
                _ => panic!("'rustfmt' should have failed due to bad source code"),
            }
        })
    }
}
