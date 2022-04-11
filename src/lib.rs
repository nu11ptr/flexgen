pub mod config;
pub mod var;

use std::collections::HashMap;
use std::io::Write;
use std::{fs, io};

use flexstr::SharedStr;
use heck::ToSnakeCase;
use proc_macro2::TokenStream;
use quote::quote;
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};
use rust_format::{Formatter, PostProcess, PrettyPlease};

use crate::config::{Config, FragmentItem};
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
    #[error("The configuration file item '{0}' doesn't exist")]
    FileNotFound(SharedStr),
    #[error("The configuration fragment list item '{0}' doesn't exist")]
    FragmentListNotFound(SharedStr),
    #[error("Errors occurred during execution: {0:?}")]
    ExecutionErrors(Vec<CodeGenError>),
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

struct FileGenerator<'exec> {
    name: &'exec SharedStr,
    vars: TokenVars,
    fragments: &'exec CodeFragments,
    config: &'exec Config,
}

impl<'exec> FileGenerator<'exec> {
    fn new(
        name: &'exec SharedStr,
        fragments: &'exec CodeFragments,
        config: &'exec Config,
    ) -> Result<Self, CodeGenError> {
        // Get merged vars
        let vars = config.vars(name)?;

        Ok(Self {
            name,
            vars,
            fragments,
            config,
        })
    }

    fn assemble_source(results: Vec<TokenStream>) -> Result<String, CodeGenError> {
        let tokens = quote! { #( #results )* };

        let config = rust_format::Config::new_str().post_proc(PostProcess::ReplaceMarkers);
        let formatter = PrettyPlease::from_config(config);

        // TODO: Optional secondary format with `rustfmt`

        Ok(formatter.format_tokens(tokens)?)
    }

    fn build_source(
        &self,
        fragments: &[FragmentItem],
        exceptions: &[SharedStr],
        results: &mut Vec<TokenStream>,
    ) -> Result<(), CodeGenError> {
        for (idx, fragment) in fragments.iter().enumerate() {
            match fragment {
                FragmentItem::FragmentListRef(name) => {
                    if exceptions.contains(name) {
                        continue;
                    }

                    let fragments = self.config.fragment_list(name)?;
                    return self.build_source(fragments, exceptions, results);
                }
                FragmentItem::Fragment(name) => {
                    if exceptions.contains(name) {
                        continue;
                    }

                    // Panic safety: This was pre-validated
                    let fragment = self.fragments[name];
                    let tokens = fragment.generate(&self.vars)?;
                    results.push(tokens);

                    // Push a blank line on all but the last fragment in the list
                    if idx < fragments.len() - 1 {
                        results.push(quote! { _blank_!(); })
                    }
                }
            }
        }

        Ok(())
    }

    fn generate_string(&self) -> Result<(SharedStr, String), CodeGenError> {
        // TODO: Combine into one call?
        let fragments = self.config.file_fragment_list(self.name)?;
        let exceptions = self.config.file_fragment_exceptions(self.name)?;

        // TODO: What capacity? (we could have nested lists, etc.)
        let mut results = Vec::with_capacity(self.fragments.len() * 2);
        // Would be nice to make this a constant, but _comment_! marker needs a literal
        let comment = quote! {
            _comment_!("WARNING: This file has been auto-generated using flexgen");
            _comment_!("https://github.com/nu11ptr/flexgen).");
            _comment_!("Any manual modifications to this file will be overwritten ");
            _comment_!("the next time this file is generated.");
            _blank_!();
        };
        results.push(comment);

        self.build_source(fragments, exceptions, &mut results)?;
        let source = Self::assemble_source(results)?;

        Ok((self.name.clone(), source))
    }

    fn generate_file(&self) -> Result<(), CodeGenError> {
        let (_, source) = self.generate_string()?;

        let mut file = fs::File::create(self.config.file_path(self.name)?)?;
        file.write_all(source.as_bytes())?;
        Ok(())
    }
}

pub struct CodeGenerator {
    code: CodeFragments,
    config: Config,
}

impl CodeGenerator {
    #[inline]
    pub fn new(code: CodeFragments, mut config: Config) -> Result<Self, CodeGenError> {
        config.build_and_validate(&code)?;
        Ok(Self { code, config })
    }

    fn parse_results<T>(results: Vec<Result<T, CodeGenError>>) -> Result<Vec<T>, CodeGenError> {
        let mut errors = Vec::with_capacity(results.len());
        let mut source = Vec::with_capacity(results.len());

        for result in results {
            match result {
                Ok(result) => source.push(result),
                Err(err) => errors.push(err),
            }
        }

        if errors.is_empty() {
            Ok(source)
        } else {
            Err(CodeGenError::ExecutionErrors(errors))
        }
    }

    fn generate(&self, to_file: bool) -> Result<HashMap<SharedStr, String>, CodeGenError> {
        let names = self.config.file_names();

        Ok(if to_file {
            let results: Vec<Result<_, _>> = names
                .par_iter()
                .map(|&name| FileGenerator::new(name, &self.code, &self.config)?.generate_file())
                .collect();

            Self::parse_results(results)?;
            HashMap::new()
        } else {
            let results: Vec<Result<_, _>> = names
                .par_iter()
                .map(|&name| FileGenerator::new(name, &self.code, &self.config)?.generate_string())
                .collect();
            let results: HashMap<_, _> = Self::parse_results(results)?.into_iter().collect();
            results
        })
    }

    #[inline]
    pub fn generate_strings(&self) -> Result<HashMap<SharedStr, String>, CodeGenError> {
        self.generate(false)
    }

    #[inline]
    pub fn generate_files(&self) -> Result<(), CodeGenError> {
        self.generate(true).map(|_| ())
    }
}

// *** Misc. Types ***

pub type CodeFragments = HashMap<SharedStr, &'static (dyn CodeFragment + Send + Sync)>;

/// A single code fragment - the smallest unit of work
pub trait CodeFragment {
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, CodeGenError>;
}
