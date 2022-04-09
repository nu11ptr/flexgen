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

mod replace;

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

use std::collections::HashMap;
use std::default::Default;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{env, fmt, fs, io, string};

const RUST_FMT: &str = "rustfmt";
const RUST_FMT_KEY: &str = "RUSTFMT";

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

// *** Config ***

/// The configuration for the formatters. Other than edition, these options should be considered
/// proprietary to the formatter being used. They are not portable between formatters.
///
/// Currently, only [RustFmt] uses this and [PrettyPlease] doesn't take any configuration
#[derive(Clone, Debug, Default)]
pub struct Config<K, V> {
    edition: Edition,
    options: HashMap<K, V>,
}

impl<K, V> Config<K, V> {
    /// Creates a new configuration from the given edition and options
    #[inline]
    pub fn new(edition: Edition, options: HashMap<K, V>) -> Self {
        Self { edition, options }
    }
}

// *** Formatter ***

/// A unified interface to all formatters. It allows for formatting from string, file, or
/// [TokenStream](proc_macro2::TokenStream) (feature `token_stream` required)
pub trait Formatter {
    /// Format the given string and return the results in another `String`. An error is returned
    /// if any issues occur during formatting
    fn format_str(&self, source: impl AsRef<str>) -> Result<String, Error>;

    /// Format the given file specified hte path and overwrite the file with the results. An error
    /// is returned if any issues occur during formatting
    fn format_file(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        // Read our file into a string
        let mut file = fs::File::open(path.as_ref())?;
        let len = file.metadata()?.len();
        let mut source = String::with_capacity(len as usize);

        file.read_to_string(&mut source)?;
        // Close the file now that we are done with it
        drop(file);

        let result = self.format_str(source)?;

        let mut file = fs::File::create(path)?;
        file.write_all(result.as_bytes())?;
        Ok(())
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
/// use std::collections::HashMap;
/// use rust_format::{Config, Edition, Formatter, RustFmt};
///
/// let source = r#"use std::marker; use std::io; mod test; mod impls;"#;
///
/// let mut options = HashMap::with_capacity(2);
/// options.insert("reorder_imports", "false");
/// options.insert("reorder_modules", "false");
///
/// let config = Config::new(Edition::Rust2018, options);
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
pub struct RustFmt {
    rustfmt: OsString,
    config_str: Option<OsString>,
    edition: Edition,
}

impl RustFmt {
    #[inline]
    fn new() -> Self {
        Self::build(None as Option<Config<&OsStr, &OsStr>>)
    }

    /// Creates a new instance of the formatter from the given configuration
    #[inline]
    pub fn from_config<K, V>(config: Config<K, V>) -> Self
    where
        K: Default + AsRef<OsStr>,
        V: Default + AsRef<OsStr>,
    {
        Self::build(Some(config))
    }

    fn build<K, V>(config: Option<Config<K, V>>) -> Self
    where
        K: Default + AsRef<OsStr>,
        V: Default + AsRef<OsStr>,
    {
        // Use 'rustfmt' specified by the environment var, if specified, else use the default
        // Safety: constant - always succeeds
        let rustfmt = env::var_os(RUST_FMT_KEY).unwrap_or_else(|| RUST_FMT.parse().unwrap());

        let config = config.unwrap_or_default();
        let edition = config.edition;
        let config_str = Self::build_config_str(config.options);
        Self {
            rustfmt,
            config_str,
            edition,
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
        let mut proc = Command::new(&self.rustfmt)
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
            Ok(stdout)
        } else {
            Err(Error::BadSourceCode(stderr))
        }
    }

    fn format_file(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let args = self.build_args(Some(path.as_ref()));

        // Launch rustfmt
        let proc = Command::new(&self.rustfmt)
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

// *** Pretty Please ***

/// This formatter uses [prettyplease](https://crates.io/crates/prettyplease) for formatting source code
///
/// From string:
/// ```
/// use rust_format::{Formatter, PrettyPlease};
///
/// let source = r#"fn main() { println!("Hello World!"); }"#;
///
/// let actual = PrettyPlease.format_str(source).unwrap();
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
/// let actual = PrettyPlease.format_tokens(source).unwrap();
/// let expected = r#"fn main() {
///     println!("Hello World!");
/// }
/// "#;
///
/// assert_eq!(expected, actual);
/// ```
#[cfg(feature = "prettyplease")]
#[cfg_attr(docsrs, doc(cfg(feature = "prettyplease")))]
pub struct PrettyPlease;

#[cfg(feature = "prettyplease")]
#[cfg_attr(docsrs, doc(cfg(feature = "prettyplease")))]
impl Formatter for PrettyPlease {
    #[inline]
    fn format_str(&self, source: impl AsRef<str>) -> Result<String, Error> {
        let f = syn::parse_file(source.as_ref())?;
        Ok(prettyplease::unparse(&f))
    }

    #[inline]
    #[cfg(feature = "token_stream")]
    #[cfg_attr(docsrs, doc(cfg(feature = "token_stream")))]
    fn format_tokens(&self, tokens: proc_macro2::TokenStream) -> Result<String, Error> {
        let f = syn::parse2::<syn::File>(tokens)?;
        Ok(prettyplease::unparse(&f))
    }
}

// *** Tests ***

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, Write};

    #[cfg(feature = "prettyplease")]
    use crate::PrettyPlease;
    use crate::{Error, Formatter, RustFmt};

    fn format_file(fmt: impl Formatter) {
        // Write source code to file
        let source = r#"fn main() { println!("Hello World!"); }"#;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(source.as_bytes()).unwrap();

        fmt.format_file(file.path()).unwrap();

        // Now read back the formatted file
        file.rewind().unwrap();
        let mut actual = String::with_capacity(128);
        file.read_to_string(&mut actual).unwrap();

        let expected = r#"fn main() {
    println!("Hello World!");
}
"#;

        assert_eq!(expected, actual);
    }

    #[test]
    fn rustfmt_file() {
        format_file(RustFmt::new());
    }

    #[cfg(feature = "prettyplease")]
    #[test]
    fn prettyplease_file() {
        format_file(PrettyPlease);
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
        bad_format_file(RustFmt::new());
    }

    #[cfg(feature = "prettyplease")]
    #[test]
    fn prettyplease_bad_file() {
        bad_format_file(PrettyPlease);
    }
}
