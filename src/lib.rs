pub mod config;
pub mod var;

use std::collections::HashMap;
use std::io;

use flexstr::SharedStr;
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use crate::config::Config;
use crate::var::TokenVars;

#[doc(hidden)]
#[inline]
pub fn make_key(s: &'static str) -> SharedStr {
    SharedStr::from_ref(&s.to_snake_case())
}

#[macro_export]
macro_rules! register_fragments {
    (%item%, $v:ident) => { () };
    (%count%, $($v:ident),+) => { [$($crate::register_fragments!(%item%, $v)),+].len() };
    // Allow trailing comma
    ($($fragment:ident,)+) => { $crate::register_fragments!($($fragment),+) };
    ($($fragment:ident),+) => {
        {
            let cap = $crate::register_fragments!(%count%, $($fragment),+);
            let mut map = $crate::CodeFragments::with_capacity(cap);

            $(
                map.insert($crate::make_key(stringify!($fragment)), &$fragment);
            )+
            map
        }
    };
}

// *** CodeGenError ***

#[derive(Debug, thiserror::Error)]
pub enum CodeGenError {
    #[error("The specified variable '{0}' was missing.")]
    MissingVar(SharedStr),
    #[error("These code fragments from the configuration are missing: {0:?}")]
    MissingFragments(Vec<SharedStr>),
    #[error("The fragment list '{0}' referenced by file '{1}' doesn't exist")]
    MissingFragmentList(SharedStr, SharedStr),
    #[error("These fragment list exceptions referenced by file '{1}' don't exist: {0:?}")]
    MissingFragmentListExceptions(Vec<SharedStr>, SharedStr),
    #[error("The specified item was a 'list' instead of a 'single' item (or vice versa)")]
    WrongItem,
    #[error("The code item could not be parsed: {0}")]
    UnrecognizedCodeItem(#[from] syn::Error),
    #[error("The item did not match any known code item prefix: {0}")]
    NotCodeItem(SharedStr),
    #[error("There was an error while deserializing: {0}")]
    DeserializeError(String),

    #[error(transparent)]
    FormatError(#[from] rust_format::Error),
    #[error(transparent)]
    IOError(#[from] io::Error),
    #[error(transparent)]
    TOMLError(#[from] toml::de::Error),
}

// *** Execute ***

fn execute_file_to_string(
    _name: &SharedStr,
    _fragments: &CodeFragments,
    _config: &Config,
) -> Result<(SharedStr, String), CodeGenError> {
    todo!()
}

fn execute_file(
    _name: &SharedStr,
    _fragments: &CodeFragments,
    _config: &Config,
) -> Result<(), CodeGenError> {
    todo!()
}

fn do_execute(
    to_file: bool,
    code: &CodeFragments,
    config: &Config,
) -> Result<HashMap<SharedStr, String>, CodeGenError> {
    let names = config.file_names();

    let results = if to_file {
        let _results: Vec<Result<_, _>> = names
            .par_iter()
            .map(|&name| execute_file(name, code, config))
            .collect();

        HashMap::new()
    } else {
        let _results: Vec<Result<_, _>> = names
            .par_iter()
            .map(|&name| execute_file(name, code, config))
            .collect();

        let map_results = HashMap::with_capacity(names.len());

        map_results
    };

    Ok(results)
}

pub fn execute_to_strings(
    code: &CodeFragments,
    config: &Config,
) -> Result<HashMap<SharedStr, String>, CodeGenError> {
    do_execute(false, code, config)
}

pub fn execute(code: &CodeFragments, config: &Config) -> Result<(), CodeGenError> {
    do_execute(true, code, config).map(|_| ())
}

// *** Misc. Types ***

pub type CodeFragments = HashMap<SharedStr, &'static (dyn CodeFragment + Send + Sync)>;

/// A single code fragment - the smallest unit of work
pub trait CodeFragment {
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, CodeGenError>;
}
