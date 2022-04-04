pub mod config;
pub mod var;

use std::collections::HashMap;

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

#[derive(Clone, Debug, thiserror::Error)]
pub enum CodeGenError {
    #[error("The specified variable '{0}' was missing.")]
    MissingVar(SharedStr),
    #[error("The specified code fragment '{0}' was missing.")]
    MissingFragment(SharedStr),
    #[error("The specified item was a 'list' instead of a 'single' item (or vice versa)")]
    WrongItem,
    #[error("The code item could not be parsed: {0}")]
    UnrecognizedCodeItem(#[from] syn::Error),
    #[error("The item did not match any known code item prefix: {0}")]
    NotCodeItem(SharedStr),
    #[error("There was an error while deserializing: {0}")]
    DeserializeError(String),
}

// *** FragmentItem ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FragmentItem {
    // Must be first so Serde uses this one always
    Fragment(SharedStr),
    FragmentListRef(SharedStr),
}

// *** Fragment Lists ***

#[derive(Clone, Debug, Default, serde::Deserialize, PartialEq)]
pub struct FragmentLists(HashMap<SharedStr, Vec<FragmentItem>>);

impl FragmentLists {
    pub fn build(&self) -> Self {
        let mut lists = HashMap::with_capacity(self.0.len());

        for (key, fragments) in &self.0 {
            let mut new_fragments = Vec::with_capacity(fragments.len());

            for fragment in fragments {
                match fragment {
                    FragmentItem::Fragment(s) | FragmentItem::FragmentListRef(s) => {
                        // If it is also a key, that means it is a list reference
                        if self.0.contains_key(s) {
                            new_fragments.push(FragmentItem::FragmentListRef(s.clone()));
                        } else {
                            new_fragments.push(FragmentItem::Fragment(s.clone()));
                        }
                    }
                }
            }

            lists.insert(key.clone(), new_fragments);
        }

        Self(lists)
    }

    pub fn validate(&self, code: &CodeFragments) -> Result<(), CodeGenError> {
        for fragments in self.0.values() {
            let missing = fragments.iter().find(|&fragment| match fragment {
                FragmentItem::Fragment(fragment) => !code.contains_key(fragment),
                FragmentItem::FragmentListRef(_) => false,
            });

            if let Some(FragmentItem::Fragment(fragment)) = missing {
                return Err(CodeGenError::MissingFragment(fragment.clone()));
            }
        }

        Ok(())
    }
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
    // TODO: Move this into config from_reader? (but then it needs `CodeFragments`
    // Make sure every item in fragment list also has a code fragment
    config.fragment_lists.validate(code)?;

    let names: Vec<_> = config.files.keys().collect();

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
