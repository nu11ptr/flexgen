#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

//! A Rust source code formatting crate with a unified interface for string, file, and  
//! [TokenStream](proc_macro2::TokenStream) input. It currently supports
//! [rustfmt](https://crates.io/crates/rustfmt-nightly) and
//! [prettyplease](https://crates.io/crates/prettyplease).

use std::fmt::Debug;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::Path;
use std::{fmt, fs, io};

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

impl Default for Edition {
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
}

// TODO: Replace with a real implementation
impl fmt::Display for Error {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as Debug>::fmt(self, f)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    #[inline]
    fn from(err: io::Error) -> Self {
        Self::IOError(err)
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
    I: FromIterator<(K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    edition: Edition,
    options: I,
    phantom: PhantomData<(K, V)>,
}

impl<I, K, V> Config<I, K, V>
where
    I: FromIterator<(K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    /// Creates a new configuration from the given edition and options
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
    fn format_str(source: impl AsRef<str>) -> Result<String, Error>;

    /// Format the given file specified hte path and overwrite the file with the results. An error
    /// is returned if any issues occur during formatting
    fn format_file(path: impl AsRef<Path>) -> Result<(), Error> {
        // Read our file into a string
        let mut file = fs::File::open(path.as_ref())?;
        let len = file.metadata()?.len();
        let mut source = String::with_capacity(len as usize);

        file.read_to_string(&mut source)?;
        // Close the file now that we are done with it
        drop(file);

        let result = Self::format_str(source)?;

        let mut file = fs::File::create(path)?;
        file.write_all(result.as_bytes())?;
        Ok(())
    }

    /// Format the given [TokenStream](proc_macro2::TokenStream) and return the results in a `String`.
    /// An error is returned if any issues occur during formatting
    #[cfg(feature = "token_stream")]
    #[cfg_attr(docsrs, doc(cfg(feature = "token_stream")))]
    #[inline]
    fn format_tokens(tokens: proc_macro2::TokenStream) -> Result<String, Error> {
        Self::format_str(tokens.to_string())
    }
}

// *** Pretty Please ***

#[cfg(feature = "prettyplease")]
#[cfg_attr(docsrs, doc(cfg(feature = "prettyplease")))]
pub struct PrettyPlease;

#[cfg(feature = "prettyplease")]
#[cfg_attr(docsrs, doc(cfg(feature = "prettyplease")))]
impl Formatter for PrettyPlease {
    #[inline]
    fn format_str(source: impl AsRef<str>) -> Result<String, Error> {
        let f = syn::parse_file(source.as_ref())?;
        Ok(prettyplease::unparse(&f))
    }

    #[inline]
    #[cfg(feature = "token_stream")]
    #[cfg_attr(docsrs, doc(cfg(feature = "token_stream")))]
    fn format_tokens(tokens: proc_macro2::TokenStream) -> Result<String, Error> {
        let f = syn::parse2::<syn::File>(tokens)?;
        Ok(prettyplease::unparse(&f))
    }
}

#[cfg(test)]
mod tests {}
