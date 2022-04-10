#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

//! A Rust source code formatting crate with a unified interface for string, file, and
//! [TokenStream](proc_macro2::TokenStream) input. It currently supports
//! [rustfmt](https://crates.io/crates/rustfmt-nightly) and
//! [prettyplease](https://crates.io/crates/prettyplease).
//!
//! ```
//! use rust_format::{Formatter, RustFmt};
//!
//! let source = r#"fn main() { println!("Hello World!"); }"#;
//!
//! let actual = RustFmt::default().format_str(source).unwrap();
//! let expected = r#"fn main() {
//!     println!("Hello World!");
//! }
//! "#;
//!
//! assert_eq!(expected, actual);
//! ```

#[cfg(feature = "post_process")]
mod replace;

#[cfg(not(feature = "post_process"))]
mod replace {
    use std::borrow::Cow;

    use crate::Error;

    #[inline]
    pub(crate) fn replace_markers(s: &str, _replace_doc_blocks: bool) -> Result<Cow<str>, Error> {
        Ok(Cow::Borrowed(s))
    }
}

// Trick to test README samples (from: https://github.com/rust-lang/cargo/issues/383#issuecomment-720873790)
#[cfg(feature = "post_process")]
#[cfg(feature = "token_stream")]
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

use std::borrow::Cow;
use std::collections::HashMap;
use std::default::Default;
use std::ffi::{OsStr, OsString};
use std::hash::Hash;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fmt, fs, io, string};

const RUST_FMT: &str = "rustfmt";
const RUST_FMT_KEY: &str = "RUSTFMT";

// *** Marker macros ***

/// A "marker" macro used to mark locations in the source code where blank lines should be inserted.
/// If no parameter is given, one blank line is assumed, otherwise the integer literal specified
/// gives the # of blank lines to insert.
///
/// It is important to understand this is NOT actually a macro that is executed. In fact, it is just
/// here for documentation purposes. Instead, this works as a raw set of tokens in the source code
/// that we match against verbatim. This means it cannot be renamed on import for example, and it MUST be
/// invoked as `_blank_!(`, then an optional Rust integer literal, and then `);`. These are matched exactly
/// and no excess whitespace is allowed or it won't be matched.
///
/// Actually executing this macro has no effect and it is not meant to even be imported.
#[cfg(feature = "post_process")]
#[cfg_attr(docsrs, doc(cfg(feature = "post_process")))]
#[macro_export]
macro_rules! _blank_ {
    () => {};
    ($lit:literal) => {};
}

/// A "marker" macro used to mark locations in the source code where comments should be inserted.
/// If no parameter is given, a single blank comment is assumed, otherwise the string literal
/// specified is broken into lines and those comments will be inserted individually.
///
/// It is important to understand this is NOT actually a macro that is executed. In fact, it is just
/// here for documentation purposes. Instead, this works as a raw set of tokens in the source code
/// that we match against verbatim. This means it cannot be renamed on import for example, and it MUST be
/// invoked as `_comment_!(`, then an optional Rust `str` literal, and then `);`. These are matched exactly
/// and no excess whitespace is allowed or it won't be matched.
///
/// Actually executing this macro has no effect and it is not meant to even be imported.
#[cfg(feature = "post_process")]
#[cfg_attr(docsrs, doc(cfg(feature = "post_process")))]
#[macro_export]
macro_rules! _comment_ {
    () => {};
    ($lit:literal) => {};
}

// *** Error ***

/// This error is returned when errors are triggered during the formatting process
#[derive(Debug)]
pub enum Error {
    /// An I/O related error occurred
    IOError(io::Error),
    /// The response of formatting was not valid UTF8
    UTFConversionError(string::FromUtf8Error),
    /// The source code has bad syntax and could not be formatted
    BadSourceCode(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::IOError(err) => <io::Error as fmt::Display>::fmt(err, f),
            Error::UTFConversionError(err) => <string::FromUtf8Error as fmt::Display>::fmt(err, f),
            Error::BadSourceCode(cause) => {
                f.write_str("An error occurred while formatting the source code: ")?;
                f.write_str(cause)
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    #[inline]
    fn from(err: io::Error) -> Self {
        Error::IOError(err)
    }
}

impl From<string::FromUtf8Error> for Error {
    #[inline]
    fn from(err: string::FromUtf8Error) -> Self {
        Error::UTFConversionError(err)
    }
}

#[cfg(feature = "syn")]
impl From<syn::Error> for Error {
    #[inline]
    fn from(err: syn::Error) -> Self {
        Error::BadSourceCode(err.to_string())
    }
}

// *** Edition ***

/// The Rust edition the source code uses
#[derive(Clone, Copy, Debug)]
pub enum Edition {
    /// Rust 2015 edition
    Rust2015,
    /// Rust 2018 edition
    Rust2018,
    /// Rust 2021 edition
    Rust2021,
}

impl Edition {
    #[inline]
    fn as_os_str(self) -> &'static OsStr {
        match self {
            Edition::Rust2015 => "2015",
            Edition::Rust2018 => "2018",
            Edition::Rust2021 => "2021",
        }
        .as_ref()
    }
}

impl Default for Edition {
    #[inline]
    fn default() -> Self {
        Edition::Rust2021
    }
}

// *** Post Processing ***

/// Post format processing options - optionally replace comment/blank markers and doc blocks
#[derive(Clone, Copy, Debug)]
pub enum PostProcess {
    /// No post processing after formatting (default)
    None,

    /// Replace [`_blank_!`] and [`_comment_!`] markers
    #[cfg(feature = "post_process")]
    #[cfg_attr(docsrs, doc(cfg(feature = "post_process")))]
    ReplaceMarkers,

    /// Replace [`_blank_!`] and [`_comment_!`] markers and  `#[doc = ""]` (with `///`)
    #[cfg(feature = "post_process")]
    #[cfg_attr(docsrs, doc(cfg(feature = "post_process")))]
    ReplaceMarkersAndDocBlocks,
}

impl PostProcess {
    /// Returns true if blank and comment markers should be replaced in the formatted source or
    /// false if they should not be
    #[inline]
    pub fn replace_markers(self) -> bool {
        !matches!(self, PostProcess::None)
    }

    /// Returns true if doc blocks should be replaced in the formatted source or false if they
    /// should not be
    #[cfg(feature = "post_process")]
    #[inline]
    pub fn replace_doc_blocks(self) -> bool {
        matches!(self, PostProcess::ReplaceMarkersAndDocBlocks)
    }

    /// Returns true if doc blocks should be replaced in the formatted source or false if they
    /// should not be
    #[cfg(not(feature = "post_process"))]
    #[inline]
    pub fn replace_doc_blocks(self) -> bool {
        false
    }
}

impl Default for PostProcess {
    #[inline]
    fn default() -> Self {
        PostProcess::None
    }
}

// *** Config ***

/// The configuration for the formatters. Most of the options are for `rustfmt` only (they are ignored
/// by [PrettyPlease], but [PostProcess] options are used by both formatters).
#[derive(Clone, Debug, Default)]
pub struct Config<K, P, V>
where
    K: Eq + Hash + AsRef<OsStr>,
    P: Into<PathBuf>,
    V: AsRef<OsStr>,
{
    rust_fmt: Option<P>,
    edition: Edition,
    post_proc: PostProcess,
    options: HashMap<K, V>,
}

impl<'a, 'b> Config<&'a str, &'b str, &'a str> {
    /// Creates a new blank configuration with `&str` for all type params
    /// (if you wish to use different types, use [new](Config::new) instead)
    #[inline]
    pub fn new_str() -> Self {
        Self::new()
    }

    /// Creates a new configuration from the given [HashMap] of options using `&str` for all type params
    /// (if you wish to use different types, use [from_hash_map](Config::from_hash_map) instead)
    #[inline]
    pub fn from_hash_map_str(options: HashMap<&'a str, &'a str>) -> Self {
        Self::from_hash_map(options)
    }
}

impl<K, P, V> Config<K, P, V>
where
    K: Eq + Hash + AsRef<OsStr>,
    P: Into<PathBuf>,
    V: AsRef<OsStr>,
{
    /// Creates a new blank configuration without type parameter assumptions
    #[inline]
    pub fn new() -> Self {
        Self::from_hash_map(HashMap::default())
    }

    /// Creates a new configuration from the given [HashMap] of options with no type assumptions
    #[inline]
    pub fn from_hash_map(options: HashMap<K, V>) -> Self {
        Self {
            rust_fmt: None,
            edition: Edition::Rust2021,
            post_proc: PostProcess::None,
            options,
        }
    }

    /// Set the path to the `rustfmt` binary to use (`RustFmt` only, ignored by `PrettyPlease`).
    /// This takes precedence over the `RUSTFMT` environment variable, if specified
    #[inline]
    pub fn rust_fmt_path(mut self, path: P) -> Self {
        self.rust_fmt = Some(path);
        self
    }

    /// Set the Rust edition of the source input (`RustFmt` only, ignored by `PrettyPlease`)
    #[inline]
    pub fn edition(mut self, edition: Edition) -> Self {
        self.edition = edition;
        self
    }

    /// Set the post processing option after formatting (used by both `RustFmt` and `PrettyPlease`)
    #[inline]
    pub fn post_proc(mut self, post_proc: PostProcess) -> Self {
        self.post_proc = post_proc;
        self
    }

    /// Set a key/value pair option (`RustFmt` only, ignored by `PrettyPlease`).
    /// See [here](https://rust-lang.github.io/rustfmt/) for a list of possible options
    #[inline]
    pub fn option(mut self, key: K, value: V) -> Self {
        self.options.insert(key, value);
        self
    }
}

// *** Misc. format related functions ***

#[inline]
fn post_process(post_proc: PostProcess, source: String) -> Result<String, Error> {
    if post_proc.replace_markers() {
        match replace::replace_markers(&source, post_proc.replace_doc_blocks())? {
            // No change
            Cow::Borrowed(_) => Ok(source),
            // Changed
            Cow::Owned(source) => Ok(source),
        }
    } else {
        Ok(source)
    }
}

#[inline]
fn file_to_string(path: impl AsRef<Path>) -> Result<String, Error> {
    // Read our file into a string
    let mut file = fs::File::open(path.as_ref())?;
    let len = file.metadata()?.len();
    let mut source = String::with_capacity(len as usize);

    file.read_to_string(&mut source)?;
    Ok(source)
}

#[inline]
fn string_to_file(path: impl AsRef<Path>, source: &str) -> Result<(), Error> {
    let mut file = fs::File::create(path)?;
    file.write_all(source.as_bytes())?;
    Ok(())
}

// *** Formatter ***

/// A unified interface to all formatters. It allows for formatting from string, file, or
/// [TokenStream](proc_macro2::TokenStream)
pub trait Formatter {
    /// Format the given string and return the results in another `String`. An error is returned
    /// if any issues occur during formatting
    fn format_str(&self, source: impl AsRef<str>) -> Result<String, Error>;

    /// Format the given file specified hte path and overwrite the file with the results. An error
    /// is returned if any issues occur during formatting
    fn format_file(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let source = file_to_string(path.as_ref())?;
        let result = self.format_str(source)?;
        string_to_file(path, &result)
    }

    /// Format the given [TokenStream](proc_macro2::TokenStream) and return the results in a `String`.
    /// An error is returned if any issues occur during formatting
    #[cfg(feature = "token_stream")]
    #[cfg_attr(docsrs, doc(cfg(feature = "token_stream")))]
    #[inline]
    fn format_tokens(&self, tokens: proc_macro2::TokenStream) -> Result<String, Error> {
        self.format_str(tokens.to_string())
    }
}

// *** Rust Fmt ***

/// This formatter uses `rustfmt` for formatting source code
///
/// An example using a custom configuration:
/// ```
/// use rust_format::{Config, Edition, Formatter, RustFmt};
///
/// let source = r#"use std::marker; use std::io; mod test; mod impls;"#;
///
/// let mut config = Config::new_str()
///     .edition(Edition::Rust2018)
///     .option("reorder_imports", "false")
///     .option("reorder_modules", "false");
/// let rustfmt = RustFmt::from_config(config);
///
/// let actual = rustfmt.format_str(source).unwrap();
/// let expected = r#"use std::marker;
/// use std::io;
/// mod test;
/// mod impls;
/// "#;
///
/// assert_eq!(expected, actual);
/// ```
#[derive(Clone)]
pub struct RustFmt {
    rust_fmt: PathBuf,
    edition: Edition,
    post_proc: PostProcess,
    config_str: Option<OsString>,
}

impl RustFmt {
    /// Creates a new instance of `RustFmt` using a default configuration
    #[inline]
    pub fn new() -> Self {
        Self::build(None as Option<Config<&OsStr, &OsStr, &OsStr>>)
    }

    /// Creates a new instance of the formatter from the given configuration
    #[inline]
    pub fn from_config<K, P, V>(config: Config<K, P, V>) -> Self
    where
        K: Default + Eq + Hash + AsRef<OsStr>,
        P: Default + Into<PathBuf>,
        V: Default + AsRef<OsStr>,
    {
        Self::build(Some(config))
    }

    fn build<K, P, V>(config: Option<Config<K, P, V>>) -> Self
    where
        K: Default + Eq + Hash + AsRef<OsStr>,
        P: Default + Into<PathBuf>,
        V: Default + AsRef<OsStr>,
    {
        let config = config.unwrap_or_default();

        // Use 'rustfmt' specified by the config first, and if not, environment var, if specified,
        // else use the default
        let rust_fmt = match config.rust_fmt {
            Some(path) => path.into(),
            None => env::var_os(RUST_FMT_KEY)
                .unwrap_or_else(|| RUST_FMT.parse().unwrap())
                .into(),
        };

        let edition = config.edition;
        let config_str = Self::build_config_str(config.options);
        Self {
            rust_fmt,
            edition,
            post_proc: config.post_proc,
            config_str,
        }
    }

    fn build_config_str<K, V>(cfg_options: HashMap<K, V>) -> Option<OsString>
    where
        K: Default + AsRef<OsStr>,
        V: Default + AsRef<OsStr>,
    {
        if !cfg_options.is_empty() {
            // Random # that should hold a few options
            let mut options = OsString::with_capacity(512);
            let iter = cfg_options.iter();

            for (idx, (k, v)) in iter.enumerate() {
                // Build a comma separated list but only between items (no trailing comma)
                if idx > 0 {
                    options.push(",");
                }
                options.push(k);
                options.push("=");
                options.push(v);
            }

            Some(options)
        } else {
            None
        }
    }

    fn build_args<'a, P>(&'a self, path: Option<&'a P>) -> Vec<&'a OsStr>
    where
        P: AsRef<Path> + ?Sized,
    {
        let mut args = match path {
            Some(path) => {
                let mut args = Vec::with_capacity(5);
                args.push(path.as_ref().as_ref());
                args
            }
            None => Vec::with_capacity(4),
        };

        args.push("--edition".as_ref());
        args.push(self.edition.as_os_str());

        if let Some(config_str) = &self.config_str {
            args.push("--config".as_ref());
            args.push(config_str);
        }

        args
    }
}

impl Default for RustFmt {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Formatter for RustFmt {
    fn format_str(&self, source: impl AsRef<str>) -> Result<String, Error> {
        let args = self.build_args(None as Option<&Path>);

        // Launch rustfmt
        let mut proc = Command::new(&self.rust_fmt)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(args)
            .spawn()?;

        // Get stdin and send our source code to it to be formatted
        // Safety: Can't panic - we captured stdin above
        let mut stdin = proc.stdin.take().unwrap();
        stdin.write_all(source.as_ref().as_bytes())?;
        // Close stdin
        drop(stdin);

        // Parse the results and return stdout/stderr
        let output = proc.wait_with_output()?;
        let stderr = String::from_utf8(output.stderr)?;

        if output.status.success() {
            let stdout = String::from_utf8(output.stdout)?;
            post_process(self.post_proc, stdout)
        } else {
            Err(Error::BadSourceCode(stderr))
        }
    }

    fn format_file(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        // Just use regular string method if doing post processing so we don't write to file twice
        if self.post_proc.replace_markers() {
            let source = file_to_string(path.as_ref())?;
            let result = self.format_str(source)?;
            string_to_file(path, &result)
        } else {
            let args = self.build_args(Some(path.as_ref()));

            // Launch rustfmt
            let proc = Command::new(&self.rust_fmt)
                .stderr(Stdio::piped())
                .args(args)
                .spawn()?;

            // Parse the results and return stdout/stderr
            let output = proc.wait_with_output()?;
            let stderr = String::from_utf8(output.stderr)?;

            if output.status.success() {
                Ok(())
            } else {
                Err(Error::BadSourceCode(stderr))
            }
        }
    }
}

// *** Pretty Please ***

/// This formatter uses [prettyplease](https://crates.io/crates/prettyplease) for formatting source code
///
/// From string:
/// ```
/// use rust_format::{Formatter, PrettyPlease};
///
/// let source = r#"fn main() { println!("Hello World!"); }"#;
///
/// let actual = PrettyPlease::default().format_str(source).unwrap();
/// let expected = r#"fn main() {
///     println!("Hello World!");
/// }
/// "#;
///
/// assert_eq!(expected, actual);
/// ```
///
/// From token stream:
/// ```
/// use quote::quote;
/// use rust_format::{Formatter, PrettyPlease};
///
/// let source = quote! { fn main() { println!("Hello World!"); } };
///
/// let actual = PrettyPlease::default().format_tokens(source).unwrap();
/// let expected = r#"fn main() {
///     println!("Hello World!");
/// }
/// "#;
///
/// assert_eq!(expected, actual);
/// ```
#[cfg(feature = "pretty_please")]
#[cfg_attr(docsrs, doc(cfg(feature = "pretty_please")))]
#[derive(Clone, Default)]
pub struct PrettyPlease {
    post_proc: PostProcess,
}

#[cfg(feature = "pretty_please")]
impl PrettyPlease {
    /// Creates a new instance of `PrettyPlease` using a default configuration
    #[inline]
    pub fn new() -> Self {
        Self::build(None as Option<Config<&OsStr, &OsStr, &OsStr>>)
    }

    /// Creates a new instance of `PrettyPlease` from the given configuration
    #[inline]
    pub fn from_config<K, P, V>(config: Config<K, P, V>) -> Self
    where
        K: Default + Eq + Hash + AsRef<OsStr>,
        P: Default + Into<PathBuf>,
        V: Default + AsRef<OsStr>,
    {
        Self::build(Some(config))
    }

    fn build<K, P, V>(config: Option<Config<K, P, V>>) -> Self
    where
        K: Default + Eq + Hash + AsRef<OsStr>,
        P: Default + Into<PathBuf>,
        V: Default + AsRef<OsStr>,
    {
        let config = config.unwrap_or_default();

        Self {
            post_proc: config.post_proc,
        }
    }

    #[inline]
    fn format(&self, f: &syn::File) -> Result<String, Error> {
        let result = prettyplease::unparse(f);
        post_process(self.post_proc, result)
    }
}

#[cfg(feature = "pretty_please")]
impl Formatter for PrettyPlease {
    #[inline]
    fn format_str(&self, source: impl AsRef<str>) -> Result<String, Error> {
        let f = syn::parse_file(source.as_ref())?;
        self.format(&f)
    }

    #[inline]
    #[cfg(feature = "token_stream")]
    #[cfg_attr(docsrs, doc(cfg(feature = "token_stream")))]
    fn format_tokens(&self, tokens: proc_macro2::TokenStream) -> Result<String, Error> {
        let f = syn::parse2::<syn::File>(tokens)?;
        self.format(&f)
    }
}

// *** Tests ***

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, Write};

    use pretty_assertions::assert_eq;

    #[cfg(feature = "post_process")]
    use crate::PostProcess;
    #[cfg(feature = "pretty_please")]
    use crate::PrettyPlease;
    use crate::{Config, Error, Formatter, RustFmt, RUST_FMT, RUST_FMT_KEY};

    const PLAIN_EXPECTED: &str = r#"#[doc = " This is main"]
fn main() {
    _comment_!("This prints hello world");
    println!("Hello World!");
    _blank_!();
}
"#;
    #[cfg(feature = "pretty_please")]
    const PLAIN_PP_EXPECTED: &str = r#"/// This is main
fn main() {
    _comment_!("This prints hello world");
    println!("Hello World!");
    _blank_!();
}
"#;
    #[cfg(feature = "post_process")]
    const REPLACE_EXPECTED: &str = r#"#[doc = " This is main"]
fn main() {
    // This prints hello world
    println!("Hello World!");

}
"#;
    #[cfg(feature = "post_process")]
    const REPLACE_BLOCKS_EXPECTED: &str = r#"/// This is main
fn main() {
    // This prints hello world
    println!("Hello World!");

}
"#;

    #[test]
    fn rustfmt_bad_env_path() {
        temp_env::with_var(
            RUST_FMT_KEY,
            Some("this_is_never_going_to_be_a_valid_executable"),
            || match RustFmt::new().format_str("bogus") {
                Err(Error::IOError(_)) => {}
                _ => panic!("'rustfmt' should have failed due to bad path"),
            },
        );
    }

    #[test]
    fn rustfmt_bad_config_path() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            let config =
                Config::new_str().rust_fmt_path("this_is_never_going_to_be_a_valid_executable");
            match RustFmt::from_config(config).format_str("bogus") {
                Err(Error::IOError(_)) => {}
                _ => panic!("'rustfmt' should have failed due to bad path"),
            }
        });
    }

    fn format_file(fmt: impl Formatter, expected: &str) {
        // Write source code to file
        let source = r#"#[doc = " This is main"] fn main() { _comment_!("This prints hello world");
            println!("Hello World!"); _blank_!(); }"#;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(source.as_bytes()).unwrap();

        fmt.format_file(file.path()).unwrap();

        // Now read back the formatted file
        file.rewind().unwrap();
        let mut actual = String::with_capacity(128);
        file.read_to_string(&mut actual).unwrap();

        assert_eq!(expected, actual);
    }

    #[test]
    fn rustfmt_file() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            format_file(RustFmt::new(), PLAIN_EXPECTED);
        });
    }

    // prettyplease replaces doc blocks by default
    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplease_file() {
        format_file(PrettyPlease::new(), PLAIN_PP_EXPECTED);
    }

    #[cfg(feature = "post_process")]
    #[test]
    fn rustfmt_file_replace_markers() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            let config = Config::new_str().post_proc(PostProcess::ReplaceMarkers);
            format_file(RustFmt::from_config(config), REPLACE_EXPECTED);
        });
    }

    // prettyplease replaces doc blocks by default
    #[cfg(feature = "post_process")]
    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplease_file_replace_markers() {
        let config = Config::new_str().post_proc(PostProcess::ReplaceMarkers);
        format_file(PrettyPlease::from_config(config), REPLACE_BLOCKS_EXPECTED);
    }

    #[cfg(feature = "post_process")]
    #[test]
    fn rustfmt_file_replace_markers_and_docs() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            let config = Config::new_str().post_proc(PostProcess::ReplaceMarkersAndDocBlocks);
            format_file(RustFmt::from_config(config), REPLACE_BLOCKS_EXPECTED);
        });
    }

    #[cfg(feature = "post_process")]
    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplease_file_replace_markers_and_docs() {
        let config = Config::new_str().post_proc(PostProcess::ReplaceMarkersAndDocBlocks);
        format_file(PrettyPlease::from_config(config), REPLACE_BLOCKS_EXPECTED);
    }

    fn bad_format_file(fmt: impl Formatter) {
        // Write source code to file
        let source = r#"use"#;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(source.as_bytes()).unwrap();

        match fmt.format_file(file.path()) {
            Err(Error::BadSourceCode(_)) => {}
            _ => panic!("Expected bad source code"),
        }
    }

    #[test]
    fn rustfmt_bad_file() {
        temp_env::with_var(RUST_FMT_KEY, Some(RUST_FMT), || {
            bad_format_file(RustFmt::new());
        });
    }

    #[cfg(feature = "pretty_please")]
    #[test]
    fn prettyplease_bad_file() {
        bad_format_file(PrettyPlease::new());
    }
}
