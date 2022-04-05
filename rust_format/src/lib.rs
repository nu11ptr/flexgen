use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::Path;
use std::{fs, io};

// *** Edition ***

/// The Rust edition the source code uses
#[derive(Clone, Copy, Debug)]
pub enum Edition {
    Rust2015,
    Rust2018,
    Rust2021,
}

impl Default for Edition {
    fn default() -> Self {
        Edition::Rust2021
    }
}

// *** Error ***

/// This error is returned upon any formatting errors
pub enum Error {
    IOError(io::Error),
}

impl From<io::Error> for Error {
    #[inline]
    fn from(err: io::Error) -> Self {
        Self::IOError(err)
    }
}

// *** Config ***

/// The configuration for the formatters. Other than edition, these options should be considered
/// proprietary to the formatter being used. They are not portable between formatters.
///
/// Currently, only `rustfmt` uses this and `prettyplease` silently ignores all configuration options
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
    pub fn new(edition: Edition, options: I) -> Self {
        Self {
            edition,
            options,
            phantom: PhantomData,
        }
    }
}

// *** Format Provider ***

fn file_to_string(path: impl AsRef<Path>) -> Result<String, Error> {
    // Read our file into a string
    let mut file = fs::File::open(path.as_ref())?;
    let len = file.metadata()?.len();
    let mut buffer = String::with_capacity(len as usize);

    file.read_to_string(&mut buffer)?;
    Ok(buffer)
}

/// A unified interface to all formatters. It allows for formatting from string, file, or
/// [TokenStream](proc_macro2::TokenStream) (feature `token_stream` required)
trait FormatProvider {
    fn from_str<I, K, S, V>(
        source: S,
        config: Option<&Config<I, K, V>>,
    ) -> Result<(String, bool), Error>
    where
        I: FromIterator<(K, V)>,
        K: AsRef<str>,
        S: AsRef<str>,
        V: AsRef<str>;

    #[inline]
    fn from_file<I, K, P, V>(path: P, config: Option<&Config<I, K, V>>) -> Result<bool, Error>
    where
        I: FromIterator<(K, V)>,
        K: AsRef<str>,
        P: AsRef<Path>,
        V: AsRef<str>,
    {
        let source = file_to_string(path.as_ref())?;
        let (result, changes) = Self::from_str(source, config)?;

        // Only overwrite file if there were changes
        if changes {
            let mut file = fs::File::create(path)?;
            file.write_all(result.as_bytes())?;
        }

        Ok(changes)
    }

    #[cfg(feature = "token_stream")]
    #[inline]
    fn from_token_stream<I, K, V>(
        tokens: &proc_macro2::TokenStream,
        config: Option<&Config<I, K, V>>,
    ) -> Result<String, Error>
    where
        I: FromIterator<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let (result, _) = Self::from_str(tokens.to_string(), config)?;
        Ok(result)
    }

    #[inline]
    fn from_str_check<I, K, S, V>(
        source: S,
        config: Option<&Config<I, K, V>>,
    ) -> Result<bool, Error>
    where
        I: FromIterator<(K, V)>,
        K: AsRef<str>,
        S: AsRef<str>,
        V: AsRef<str>,
    {
        let (_, changed) = Self::from_str(source, config)?;
        Ok(changed)
    }

    #[inline]
    fn from_file_check<I, K, P, V>(path: P, config: Option<&Config<I, K, V>>) -> Result<bool, Error>
    where
        I: FromIterator<(K, V)>,
        K: AsRef<str>,
        P: AsRef<Path>,
        V: AsRef<str>,
    {
        let source = file_to_string(path.as_ref())?;
        let (_, changes) = Self::from_str(source, config)?;
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {}
