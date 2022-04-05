#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

//! A Rust source code formatting crate with a unified interface for string, file, and  
//! [TokenStream](proc_macro2::TokenStream) input. It currently supports
//! [rustfmt](https://crates.io/crates/rustfmt-nightly) and
//! [prettyplease](https://crates.io/crates/prettyplease).

use std::default::Default;
use std::io::{Read, Write};
use std::marker::PhantomData;
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
    fn as_str(self) -> &'static str {
        match self {
            Edition::Rust2015 => "2015",
            Edition::Rust2018 => "2018",
            Edition::Rust2021 => "2021",
        }
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
    /// An error occurred while attempting to parse the source code - most likely bad syntax
    #[cfg(feature = "prettyplease")]
    #[cfg_attr(docsrs, doc(cfg(feature = "prettyplease")))]
    SynError(syn::Error),
    /// The response of formatting was not valid UTF8
    UTFConversionError(string::FromUtf8Error),
    /// The source code has bad syntax and could not be formatted
    BadSourceCode(String),
}

// TODO: Replace with a real implementation
impl fmt::Display for Error {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    #[inline]
    fn from(err: io::Error) -> Self {
        Self::IOError(err)
    }
}

impl From<string::FromUtf8Error> for Error {
    #[inline]
    fn from(err: string::FromUtf8Error) -> Self {
        Self::UTFConversionError(err)
    }
}

#[cfg(feature = "prettyplease")]
#[cfg_attr(docsrs, doc(cfg(feature = "prettyplease")))]
impl From<syn::Error> for Error {
    #[inline]
    fn from(err: syn::Error) -> Self {
        Self::SynError(err)
    }
}

// *** Config ***

/// The configuration for the formatters. Other than edition, these options should be considered
/// proprietary to the formatter being used. They are not portable between formatters.
///
/// Currently, only [RustFmt] uses this and [PrettyPlease] doesn't take any configuration
#[derive(Clone, Debug, Default)]
pub struct Config<I, K, V>
where
    I: Default + IntoIterator,
    K: AsRef<str>,
    V: AsRef<str>,
{
    edition: Edition,
    options: I,
    phantom: PhantomData<(K, V)>,
}

impl<I, K, V> Config<I, K, V>
where
    I: Default + IntoIterator,
    K: AsRef<str>,
    V: AsRef<str>,
{
    /// Creates a new configuration from the given edition and options
    #[inline]
    pub fn new(edition: Edition, options: I) -> Self {
        Self {
            edition,
            options,
            phantom: PhantomData,
        }
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
pub struct RustFmt<I, K, V>
where
    I: Default + IntoIterator,
    K: AsRef<str>,
    V: AsRef<str>,
{
    config: Config<I, K, V>,
}

impl<'a, I, K, V> RustFmt<I, K, V>
where
    I: Copy + Default + IntoIterator<Item = (&'a K, &'a V)>,
    K: Default + AsRef<str> + 'a,
    V: Default + AsRef<str> + 'a,
{
    /// Creates a new instance of the formatter from the given configuration
    #[inline]
    pub fn new(config: Option<Config<I, K, V>>) -> Self {
        let config = config.unwrap_or_default();
        Self { config }
    }

    fn build_config_str(&self) -> String {
        // Random # that should hold a few options
        let mut options = String::with_capacity(512);
        let iter = self.config.options.into_iter();

        for (idx, (k, v)) in iter.enumerate() {
            if idx > 0 {
                options.push(',');
            }
            options.push_str(k.as_ref());
            options.push('=');
            options.push_str(v.as_ref());
        }

        options
    }
}

impl<'a, I, K, V> Formatter for RustFmt<I, K, V>
where
    I: Copy + Default + IntoIterator<Item = (&'a K, &'a V)>,
    K: Default + AsRef<str> + 'a,
    V: Default + AsRef<str> + 'a,
{
    fn format_str(&self, source: impl AsRef<str>) -> Result<String, Error> {
        // Use 'rustfmt' specified by the environment var, if specified, else use the default
        let rustfmt = env::var(RUST_FMT_KEY).unwrap_or_else(|_| RUST_FMT.to_string());

        let mut args = Vec::with_capacity(4);
        args.push("--edition");
        args.push(self.config.edition.as_str());

        let config_str = self.build_config_str();
        if !config_str.is_empty() {
            args.push("--config");
            args.push(&config_str);
        }

        // Launch rustfmt
        let mut proc = Command::new(&rustfmt)
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
}

// *** Pretty Please ***

/// This formatter uses `prettyplease` for formatting source code
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

#[cfg(test)]
mod tests {}
