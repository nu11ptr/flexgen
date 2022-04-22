//! A flexible, yet simple quote-based code generator for creating beautiful Rust code

#![warn(missing_docs)]

/// Configuration related items
pub mod config;
/// Configuration variable related items
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
use use_builder::{UseBuilder, UseItems};

use crate::config::{Config, FragmentItem};
use crate::var::TokenVars;

#[doc(hidden)]
#[inline]
pub fn make_key(s: &'static str) -> SharedStr {
    SharedStr::from_ref(&s.to_snake_case())
}

/// Register code fragments in preparation for code generation
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

// *** Error ***

/// This error will be returned if any issues arise during the code generation process
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A variable specified in [import_vars] could not be found
    #[error("The specified variable '{0}' was missing.")]
    MissingVar(SharedStr),

    /// A fragment specified by the [CodeFragments] could not be found
    #[error("These code fragments from the configuration are missing: {0:?}")]
    MissingFragments(Vec<SharedStr>),

    /// The fragment list specified in the file section doesn't exist
    #[error("The fragment list '{0}' referenced by file '{1}' doesn't exist")]
    MissingFragmentList(SharedStr, SharedStr),

    /// The fragment list exceptions specified by the file don't exist
    #[error("These fragment list exceptions referenced by file '{1}' don't exist: {0:?}")]
    MissingFragmentListExceptions(Vec<SharedStr>, SharedStr),

    /// The file section requested from the [Config](config::Config) doesn't exist
    #[error("The configuration file item '{0}' doesn't exist")]
    FileNotFound(SharedStr),

    /// The fragment list requested from the [Config](config::Config) doesn't exist
    #[error("The configuration fragment list item '{0}' doesn't exist")]
    FragmentListNotFound(SharedStr),

    /// A nested list of execution errors occurred while trying to generate source code
    #[error("Errors occurred during execution: {0:?}")]
    ExecutionErrors(Vec<Error>),

    /// The item imported was of the wrong type (either single when a list was needed or vice versa)
    #[error("The specified item was a 'list' instead of a 'single' item (or vice versa)")]
    WrongItem,

    /// Unable to parse source code value from variable
    #[error("The code item could not be parsed: {0}")]
    UnrecognizedCodeItem(#[from] syn::Error),

    /// The code item variable data was in an unknown format
    #[error("The item did not match any known code item prefix: {0}")]
    NotCodeItem(SharedStr),

    /// An error occurred while deserializing the [Config](config::Config)
    #[error("There was an error while deserializing: {0}")]
    DeserializeError(String),

    /// An error occurred while formatting
    #[error(transparent)]
    FormatError(#[from] rust_format::Error),

    /// A general I/O error occurred
    #[error(transparent)]
    IOError(#[from] io::Error),

    /// A TOML syntax error occurred
    #[error(transparent)]
    TOMLError(#[from] toml::de::Error),

    /// An error occurred while parsing use sections
    #[error(transparent)]
    UseBuilderError(#[from] use_builder::Error),
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
    ) -> Result<Self, Error> {
        // Get merged vars
        let vars = config.vars(name)?;

        Ok(Self {
            name,
            vars,
            fragments,
            config,
        })
    }

    fn assemble_source(
        &self,
        results: Vec<TokenStream>,
        top_results: Vec<TokenStream>,
        uses: Vec<UseItems>,
    ) -> Result<String, Error> {
        // Would be nice to make this a constant, but _comment_! marker needs a literal
        let comment = quote! {
            _comment_!("+-------------------------------------------------------------------------------------------------+");
            _comment_!("| WARNING: This file has been auto-generated using FlexGen (https://github.com/nu11ptr/flexgen).  |");
            _comment_!("| Any manual modifications to this file will be overwritten the next time this file is generated. |");
            _comment_!("+-------------------------------------------------------------------------------------------------+");
        };

        let builder = UseBuilder::from_uses(uses);
        let (std_uses, ext_uses, crate_uses) = builder.into_items_sections()?;

        let tokens = quote! {
            #comment
            _blank_!();

            #( #top_results )*

            #( #std_uses )*
            _blank_!();
            #( #ext_uses )*
            _blank_!();
            #( #crate_uses )*
            _blank_!();

            #( #results )*
        };

        let config = rust_format::Config::new_str().post_proc(PostProcess::ReplaceMarkers);
        let formatter = PrettyPlease::from_config(config);
        let source = formatter.format_tokens(tokens)?;

        // Either return after PrettyPlease format or do one last final RustFmt run
        Ok(match self.config.build_rust_fmt() {
            Some(rust_fmt) => rust_fmt.format_str(source)?,
            None => source,
        })
    }

    fn build_source(
        &self,
        fragments: &[FragmentItem],
        exceptions: &[SharedStr],
        results: &mut Vec<TokenStream>,
        top_results: &mut Vec<TokenStream>,
        use_trees: &mut Vec<UseItems>,
    ) -> Result<(), Error> {
        for (idx, fragment) in fragments.iter().enumerate() {
            match fragment {
                FragmentItem::FragmentListRef(name) => {
                    if exceptions.contains(name) {
                        continue;
                    }

                    let fragments = self.config.fragment_list(name)?;
                    return self.build_source(
                        fragments,
                        exceptions,
                        results,
                        top_results,
                        use_trees,
                    );
                }
                FragmentItem::Fragment(name) => {
                    if exceptions.contains(name) {
                        continue;
                    }

                    // Panic safety: This was pre-validated
                    let fragment = self.fragments[name];
                    let tokens = fragment.generate(&self.vars)?;
                    if !tokens.is_empty() {
                        results.push(tokens);
                    }

                    let top_tokens = fragment.generate_top(&self.vars)?;
                    if !top_tokens.is_empty() {
                        top_results.push(top_tokens);
                    }

                    // Store the use tree, if we had one
                    let use_tokens = fragment.uses(&self.vars)?;
                    if !use_tokens.is_empty() {
                        use_trees.push(syn::parse2(use_tokens)?)
                    }

                    // Push a blank line on all but the last fragment in the list
                    if idx < fragments.len() - 1 {
                        results.push(quote! { _blank_!(); })
                    }
                }
            }
        }

        Ok(())
    }

    fn generate_string(&self) -> Result<(SharedStr, String), Error> {
        // TODO: Combine into one call?
        let fragments = self.config.file_fragment_list(self.name)?;
        let exceptions = self.config.file_fragment_exceptions(self.name)?;

        // TODO: What capacity? (we could have nested lists, etc.)
        let mut results = Vec::with_capacity(self.fragments.len() * 2);
        let mut top_results = Vec::with_capacity(3);
        // Random choice based on a typical file
        let mut uses = Vec::with_capacity(10);

        self.build_source(
            fragments,
            exceptions,
            &mut results,
            &mut top_results,
            &mut uses,
        )?;
        let source = self.assemble_source(results, top_results, uses)?;

        Ok((self.name.clone(), source))
    }

    fn generate_file(&self) -> Result<(), Error> {
        let (_, source) = self.generate_string()?;

        let mut file = fs::File::create(self.config.file_path(self.name)?)?;
        file.write_all(source.as_bytes())?;
        Ok(())
    }
}

/// The actual code generator
pub struct CodeGenerator {
    code: CodeFragments,
    config: Config,
}

impl CodeGenerator {
    /// Create a new instance of the `CodeGenerator`. It will validate the [Config] and return
    /// an [Error] if there are any issues
    #[inline]
    pub fn new(code: CodeFragments, mut config: Config) -> Result<Self, Error> {
        config.build_and_validate(&code)?;
        Ok(Self { code, config })
    }

    fn parse_results<T>(results: Vec<Result<T, Error>>) -> Result<Vec<T>, Error> {
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
            Err(Error::ExecutionErrors(errors))
        }
    }

    fn generate(&self, to_file: bool) -> Result<HashMap<SharedStr, String>, Error> {
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

    /// Generate the files listed in the [Config], but return them as a map of strings instead of
    /// actually writing them to he filesystem
    #[inline]
    pub fn generate_strings(&self) -> Result<HashMap<SharedStr, String>, Error> {
        self.generate(false)
    }

    /// Generate the files listed in the [Config]
    #[inline]
    pub fn generate_files(&self) -> Result<(), Error> {
        self.generate(true).map(|_| ())
    }
}

// *** Misc. Types ***

/// A map of all registered code fragments. Is is returned by a call to [register_fragments]
pub type CodeFragments = HashMap<SharedStr, &'static (dyn CodeFragment + Send + Sync)>;

/// A single code fragment - the smallest unit of work
#[allow(unused_variables)]
pub trait CodeFragment {
    /// Generate the `use` sections of the file, if any. The returned `TokenStream` can ONLY
    /// be varius `use` items. They will be deduplicated and grouped before inclusion in the file.
    #[inline]
    fn uses(&self, vars: &TokenVars) -> Result<TokenStream, Error> {
        Ok(quote! {})
    }

    /// Generate any portion of the source file that must be on the top (such as `#![]` style attributes).
    /// Each snippet will be collected in order and combined into the whole
    #[inline]
    fn generate_top(&self, vars: &TokenVars) -> Result<TokenStream, Error> {
        Ok(quote! {})
    }

    /// Generate the general body of the source file. Each snippet will be collected in order and
    /// combined into the whole
    #[inline]
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, Error> {
        Ok(quote! {})
    }
}
