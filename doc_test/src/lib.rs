#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

//! Using the [doc_test] macro, we can take any [TokenStream](proc_macro2::TokenStream) and turn it into
//! a doctest [TokenStream](proc_macro2::TokenStream) that can be interpolated in any [quote](quote::quote)
//! macro invocation.
//!
//! The [doc_comment] function takes any string and turns it into one or more comments inside a
//! [TokenStream](proc_macro2::TokenStream).
//!
//! ```
//! use quote::quote;
//! use quote_doctest::{doc_comment, doc_test, FormatDocTest};
//!
//! // Takes any `TokenStream` as input (but typically `quote` would be used)
//! let test = doc_test!(quote! {
//!     _comment_!("Calling fibonacci with 10 returns 55");
//!     assert_eq!(fibonacci(10), 55);
//!
//!     _blank_!();
//!     _comment_!("Calling fibonacci with 1 simply returns 1");
//!     assert_eq!(fibonacci(1), 1);
//! }).unwrap();
//!
//! let comment = doc_comment("This compares fib inputs and outputs:\n\n");
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
//!     /// This compares fib inputs and outputs:
//!     ///
//!     /// ```
//!     /// // Calling fibonacci with 10 returns 55
//!     /// assert_eq!(fibonacci(10), 55);
//!     ///
//!     /// // Calling fibonacci with 1 simply returns 1
//!     /// assert_eq!(fibonacci(1), 1);
//!     /// ```
//!     fn fibonacci(n: u64) -> u64 {
//!         match n {
//!             0 => 1,
//!             1 => 1,
//!             n => fibonacci(n - 1) + fibonacci(n - 2),
//!         }
//!     }
//! };
//!
//! assert_eq!(expected.format_tokens().unwrap(), actual.format_tokens().unwrap());
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

use std::cmp;

use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use rust_format::Formatter as _;

const MIN_BUFF_SIZE: usize = 128;

/// The default amount of formatter indent to remove (when generating `main`)
pub const FORMATTER_INDENT: usize = 4;

/// Creates a doctest from a [TokenStream](proc_macro2::TokenStream). Typically that is all that
/// is supplied, however, there is an optional parameter of type [DocTestOptions] that can be supplied
/// to fine tune whether or not formating is used, choose a formatter (either `pretty_please` or `rustfmt`),
/// or whether a main function generated (it is required if formatting, but one can be specified manually).
///
/// This macro returns `Result<String, Error>`. An error could be returned if an issue occurs during
/// the formatting process.
#[macro_export]
macro_rules! doc_test {
    ($tokens:expr) => {
        $crate::make_doc_test($tokens, $crate::DocTestOptions::default())
    };
    ($tokens:expr, $options:expr) => {
        $crate::make_doc_test($tokens, $options)
    };
}

pub use rust_format::{Error, _blank_, _comment_};

// *** Formatter ***

/// The formatter used to format source code - either `prettyplease` or the system `rustfmt`
#[derive(Clone)]
pub enum Formatter {
    /// Format using `prettyplease` crate
    #[cfg(feature = "pretty_please")]
    #[cfg_attr(docsrs, doc(cfg(feature = "pretty_please")))]
    PrettyPlease(rust_format::PrettyPlease),
    /// Format by calling out to the system `rustfmt`
    RustFmt(rust_format::RustFmt),
}

impl Formatter {
    /// Creates a basic default `rustfmt` `Formatter` instance that automatically strips
    /// markers from the source code
    pub fn new_rust_fmt() -> Self {
        let config =
            rust_format::Config::new_str().post_proc(rust_format::PostProcess::ReplaceMarkers);
        let rust_fmt = rust_format::RustFmt::from_config(config);
        Formatter::RustFmt(rust_fmt)
    }

    /// Creates a basic default `prettyplease` `Formatter` instance that automatically strips
    /// markers from the source code
    #[cfg(feature = "pretty_please")]
    #[cfg_attr(docsrs, doc(cfg(feature = "pretty_please")))]
    pub fn new_pretty_please() -> Self {
        let config =
            rust_format::Config::new_str().post_proc(rust_format::PostProcess::ReplaceMarkers);
        let rust_fmt = rust_format::PrettyPlease::from_config(config);
        Formatter::PrettyPlease(rust_fmt)
    }
}

#[cfg(not(feature = "pretty_please"))]
impl Default for Formatter {
    #[inline]
    fn default() -> Self {
        Formatter::new_rust_fmt()
    }
}

#[cfg(feature = "pretty_please")]
impl Default for Formatter {
    #[inline]
    fn default() -> Self {
        Formatter::new_pretty_please()
    }
}

/// Optional enum passed to [doc_test] for different configuration options
#[derive(Clone)]
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
    /// Creates a basic default `rustfmt` `DocTestOptions` instance that generates main,
    /// formats, and then strips the main function
    #[inline]
    pub fn new_rust_fmt() -> Self {
        DocTestOptions::FormatAndGenMain(Formatter::new_rust_fmt(), FORMATTER_INDENT)
    }

    /// Creates a basic default `prettyplease` `DocTestOptions` instance that generates main,
    /// formats, and then strips the main function
    #[cfg(feature = "pretty_please")]
    #[cfg_attr(docsrs, doc(cfg(feature = "pretty_please")))]
    #[inline]
    pub fn new_pretty_please() -> Self {
        DocTestOptions::FormatAndGenMain(Formatter::new_pretty_please(), FORMATTER_INDENT)
    }

    #[inline]
    fn options(self) -> (Option<Formatter>, bool, usize) {
        match self {
            DocTestOptions::NoFormatOrGenMain => (None, false, 0),
            DocTestOptions::FormatOnly(fmt) => (Some(fmt), false, 0),
            DocTestOptions::FormatAndGenMain(fmt, strip_indent) => (Some(fmt), true, strip_indent),
        }
    }
}

#[cfg(not(feature = "pretty_please"))]
impl Default for DocTestOptions {
    #[inline]
    fn default() -> Self {
        DocTestOptions::new_rust_fmt()
    }
}

#[cfg(feature = "pretty_please")]
impl Default for DocTestOptions {
    #[inline]
    fn default() -> Self {
        DocTestOptions::new_pretty_please()
    }
}

/// Attempts to translate this [TokenStream](proc_macro2::TokenStream) into a [String]. It takes an
/// optional [Formatter] which formats using either `prettyplease` or `rustfmt`. It returns a [String]
/// of the formatted code (or a single line of unformatted text, if `fmt` is `None`) or an [Error]
/// error, if one occurred.
#[inline]
fn tokens_to_string(tokens: TokenStream, fmt: Option<Formatter>) -> Result<String, Error> {
    match fmt {
        #[cfg(feature = "pretty_please")]
        Some(Formatter::PrettyPlease(pp)) => pp.format_tokens(tokens),
        Some(Formatter::RustFmt(rust_fmt)) => rust_fmt.format_tokens(tokens),
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
/// use quote_doctest::{doc_comment, FormatDocTest};
///
/// let actual = doc_comment("this\nwill be\n\nmultiple comments\n\n");
/// let expected = quote! {
///     /// this
///     /// will be
///     ///
///     /// multiple comments
///     ///
/// };
///
/// assert_eq!(expected.format_tokens().unwrap(), actual.format_tokens().unwrap());
/// ```
pub fn doc_comment(comment: impl AsRef<str>) -> TokenStream {
    let comment = comment.as_ref();

    // Unlikely to be this big, but better than reallocating
    let mut buffer = String::with_capacity(cmp::max(comment.len() * 2, MIN_BUFF_SIZE));

    // Build code from lines
    for line in comment.lines() {
        // Except for empty lines, all lines should get a space at the front
        if !line.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(line);
        buffer.push('\n');
    }

    let doc_comment: Vec<_> = buffer.lines().collect();
    quote! { #( #[doc = #doc_comment] )* }
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

    // Assemble the lines back into a string while indenting
    // NOTE: strip_indent will be zero unless gen_main was set
    let indent = " ".repeat(strip_indent);
    let doc_test = assemble_doc_test(lines, src.len(), indent);
    let doc_test: Vec<_> = doc_test.lines().collect();

    // Turn back into a token stream and into a doc test
    Ok(quote! {
        /// ```
        #( #[doc = #doc_test] )*
        /// ```
    })
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
    let mut buffer = String::with_capacity(cmp::max(cap * 2, MIN_BUFF_SIZE));

    // Build code from lines
    for mut line in lines {
        // Strip whitespace left over from main, if any (else noop)
        line = line.strip_prefix(&prefix).unwrap_or(line);

        // Except for empty lines, all lines should get a space at the front
        if !line.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(line);
        buffer.push('\n');
    }

    buffer
}

#[cfg(not(feature = "pretty_please"))]
#[inline]
fn doc_test_formatter() -> impl rust_format::Formatter {
    let config = rust_format::Config::new_str()
        .post_proc(rust_format::PostProcess::ReplaceMarkersAndDocBlocks);
    rust_format::RustFmt::from_config(config)
}

#[cfg(feature = "pretty_please")]
#[inline]
fn doc_test_formatter() -> impl rust_format::Formatter {
    rust_format::PrettyPlease::default()
}

/// Trait for converting [doc_test] results into a well formatted `String`
pub trait FormatDocTest: ToTokens {
    /// Convert results of a [doc_test] (or any other value that implements `ToTokens` that is valid
    /// Rust source) into a formatted `String`. This will also convert doc blocks (`#[doc = ""]`) into
    /// doc comments (`///`). This can be useful for display or equality testing in a unit test. An
    /// error is returned if an issue occurs during the formatting process
    ///
    /// NOTE: If `pretty_please` is not enabled then `rustfmt` will be used via the `rust_format` crate
    /// and when translating doc blocks will also translate any [`_comment_!`] or [`_blank_!`] markers.
    /// If the source of this function came from [doc_test], these will already be translated anyway,
    /// but this is mentioned for awareness.
    fn format_tokens(self) -> Result<String, Error>
    where
        Self: Sized,
    {
        // We need a function - doc blocks alone won't pass the formatter
        let doc_test = quote! {
            #self
            fn main() {}
        };

        // Format (and translate doc blocks to doc comments
        let formatter = doc_test_formatter();
        let source = formatter.format_tokens(doc_test)?;

        // Convert into lines so we trim off the last line (our added main function)
        let mut lines: Vec<_> = source.lines().collect();
        // All in one line because there will never be anything inside
        lines.pop(); // fn main() {}

        // Unlikely to be this big, but better than reallocating
        let mut buffer = String::with_capacity(cmp::max(source.len() * 2, MIN_BUFF_SIZE));

        for line in lines {
            buffer.push_str(line);
            buffer.push('\n');
        }

        buffer.shrink_to_fit();
        Ok(buffer)
    }
}

impl<T> FormatDocTest for T where T: ToTokens {}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use quote::quote;

    use crate::{
        tokens_to_string, DocTestOptions, Error, FormatDocTest, Formatter, FORMATTER_INDENT,
    };

    #[test]
    fn doctest_format() {
        let actual = quote! {
            /// ```
            /// assert_eq!(fibonacci(10), 55);
            /// assert_eq!(fibonacci(1), 1);
            /// ```
        };

        let expected = r#"/// ```
/// assert_eq!(fibonacci(10), 55);
/// assert_eq!(fibonacci(1), 1);
/// ```
"#;

        assert_eq!(expected, actual.format_tokens().unwrap());
    }

    #[test]
    fn rustfmt_format_only() {
        format_only(Formatter::new_rust_fmt());
    }

    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplz_format_only() {
        format_only(Formatter::new_pretty_please());
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
            /// ```
            /// fn main() {
            ///     assert_eq!(fibonacci(10), 55);
            ///     assert_eq!(fibonacci(1), 1);
            /// }
            /// ```
        };

        assert_eq!(
            expected.format_tokens().unwrap(),
            actual.format_tokens().unwrap()
        );
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
            /// ```
            /// fn main () { assert_eq ! (fibonacci (10) , 55) ; assert_eq ! (fibonacci (1) , 1) ; }
            /// ```
        };

        assert_eq!(
            expected.format_tokens().unwrap(),
            actual.format_tokens().unwrap()
        );
    }

    #[test]
    fn rustfmt_bad_source_code() {
        bad_source_code(Formatter::new_rust_fmt());
    }

    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplz_bad_source_code() {
        bad_source_code(Formatter::new_pretty_please());
    }

    fn bad_source_code(fmt: Formatter) {
        match tokens_to_string(quote! {"blah blah blah"}, Some(fmt)) {
            Err(Error::BadSourceCode(_)) => {}
            _ => panic!("'rustfmt' should have failed due to bad source code"),
        }
    }

    #[test]
    fn rustfmt_comment_marker() {
        comment_marker(Formatter::new_rust_fmt());
    }

    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplz_comment_marker() {
        comment_marker(Formatter::new_pretty_please());
    }

    fn comment_marker(fmt: Formatter) {
        let code = quote! {
            assert_eq!(fibonacci(10), 55);

            // Should translate to a blank line
            _comment_!();
            _comment_!("first line\n\nsecond line");
            assert_eq!(fibonacci(1), 1);
        };

        let actual = doc_test!(
            code,
            DocTestOptions::FormatAndGenMain(fmt, FORMATTER_INDENT)
        )
        .unwrap();

        let expected = quote! {
            /// ```
            /// assert_eq!(fibonacci(10), 55);
            /// //
            /// // first line
            /// //
            /// // second line
            /// assert_eq!(fibonacci(1), 1);
            /// ```
        };

        assert_eq!(
            expected.format_tokens().unwrap(),
            actual.format_tokens().unwrap()
        );
    }

    #[test]
    fn rustfmt_blank_marker() {
        blank_marker(Formatter::new_rust_fmt());
    }

    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplz_blank_marker() {
        blank_marker(Formatter::new_pretty_please());
    }

    fn blank_marker(fmt: Formatter) {
        let code = quote! {
            assert_eq!(fibonacci(10), 55);

            // Should translate to a single blank line
            _blank_!();
            assert_eq!(fibonacci(1), 1);

            // Should translate to multiple blank lines
            _blank_!(2);
        };

        let actual = doc_test!(
            code,
            DocTestOptions::FormatAndGenMain(fmt, FORMATTER_INDENT)
        )
        .unwrap();

        let expected = quote! {
            /// ```
            /// assert_eq!(fibonacci(10), 55);
            ///
            /// assert_eq!(fibonacci(1), 1);
            ///
            ///
            /// ```
        };

        assert_eq!(
            expected.format_tokens().unwrap(),
            actual.format_tokens().unwrap()
        );
    }

    #[test]
    fn rustfmt_inner_string() {
        inner_string(Formatter::new_rust_fmt());
    }

    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplz_inner_string() {
        inner_string(Formatter::new_pretty_please());
    }

    fn inner_string(fmt: Formatter) {
        let code = quote! {
            println!("inner string");
            // Escaped double quote
            println!("inner \"");
            println!("inner \r");
            println!("inner \\");

            println!(r"inner raw string");
            println!(b"inner byte string");
            println!(br"inner raw byte string");

            println!(r#"inner raw string"#);
            println!(br#"inner byte raw string"#);

            // Multiple
            println!(r#"{}"#, "multiple");

            // Raw entry fake out
            r();

            // Raw exit fake out 1
            println!(r##"inner raw " string"##);
            // Raw exit fake out 2
            println!(r##"inner raw "# string"##);
        };

        let actual = doc_test!(
            code,
            DocTestOptions::FormatAndGenMain(fmt, FORMATTER_INDENT)
        )
        .unwrap();

        let expected = quote! {
            /// ```
            /// println!("inner string");
            /// println!("inner \"");
            /// println!("inner \r");
            /// println!("inner \\");
            /// println!(r"inner raw string");
            /// println!(b"inner byte string");
            /// println!(br"inner raw byte string");
            /// println!(r#"inner raw string"#);
            /// println!(br#"inner byte raw string"#);
            /// println!(r#"{}"#, "multiple");
            /// r();
            /// println!(r##"inner raw " string"##);
            /// println!(r##"inner raw "# string"##);
            /// ```
        };

        assert_eq!(
            expected.format_tokens().unwrap(),
            actual.format_tokens().unwrap()
        );
    }
}
