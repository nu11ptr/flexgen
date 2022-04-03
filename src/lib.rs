pub mod config;

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use flexstr::{shared_str, SharedStr, ToSharedStr};
use proc_macro2::TokenStream;
use quote::ToTokens;

const IDENT: &str = "$ident$";

// *** Expand Vars ***

#[doc(hidden)]
#[inline]
pub fn import_var<'vars>(
    vars: &'vars Vars,
    var: &'static str,
) -> Result<&'vars VarValue, CodeGenError> {
    let var = shared_str!(var);
    let value = vars.get(&var).ok_or(CodeGenError::MissingVar(var))?;

    match value {
        VarItem::Single(value) => Ok(value),
        VarItem::List(_) => Err(CodeGenError::WrongItem),
    }
}

#[macro_export]
macro_rules! import_vars {
    // Allow trailing comma
    ($vars:ident => $($var:ident,)+) => { $crate::import_varss!($vars, $($var),+) };
    ($vars:ident => $($var:ident),+) => {
        $(
            let $var = $crate::import_var($vars, stringify!($var))?;
        )+
    };
}

#[doc(hidden)]
#[inline]
pub fn import_list<'vars>(
    vars: &'vars Vars,
    var: &'static str,
) -> Result<&'vars [VarValue], CodeGenError> {
    let var = shared_str!(var);
    let value = vars.get(&var).ok_or(CodeGenError::MissingVar(var))?;

    match value {
        VarItem::List(value) => Ok(value),
        VarItem::Single(_) => Err(CodeGenError::WrongItem),
    }
}

#[macro_export]
macro_rules! import_lists {
    // Allow trailing comma
    ($vars:ident => $($var:ident,)+) => { $crate::import_lists!($vars, $($var),+) };
    ($vars:ident => $($var:ident),+) => {
        $(
            let $var = $crate::import_list($vars, stringify!($var))?;
        )+
    };
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
                map.insert(flexstr::shared_str!(stringify!($fragment)), &$fragment);
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
    #[error("The specified item was a 'list' instead of a 'single' item (or vice versa)")]
    WrongItem,
    #[error("The code item could not be parsed: {0}")]
    UnrecognizedCodeItem(#[from] syn::Error),
    #[error("The item did not match any known code item prefix: {0}")]
    NotCodeItem(SharedStr),
    #[error("There was an error while deserializing: {0}")]
    DeserializeError(String),
}

// *** SynItem ***

#[derive(Clone, Debug, PartialEq)]
pub enum CodeItem {
    Ident(syn::Ident),
}

impl FromStr for CodeItem {
    type Err = CodeGenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if matches!(s.find(IDENT), Some(idx) if idx == 0) {
            let ident = syn::parse_str::<syn::Ident>(&s[IDENT.len()..])?;
            Ok(CodeItem::Ident(ident))
        } else {
            Err(CodeGenError::NotCodeItem(s.to_shared_str()))
        }
    }
}

impl ToTokens for CodeItem {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            CodeItem::Ident(ident) => ident.to_tokens(tokens),
        }
    }
}

struct SynItemVisitor;

impl<'de> serde::de::Visitor<'de> for SynItemVisitor {
    type Value = CodeItem;

    #[inline]
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string with a special prefix")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse()
            .map_err(|_| serde::de::Error::custom("Error deserializing 'str'"))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse()
            .map_err(|_| serde::de::Error::custom("Error deserializing 'String'"))
    }
}

impl<'de> serde::de::Deserialize<'de> for CodeItem {
    #[inline]
    fn deserialize<D: serde::de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(SynItemVisitor)
    }
}

// *** VarItem ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum VarItem {
    List(Vec<VarValue>),
    Single(VarValue),
}

// *** VarValue ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum VarValue {
    Number(i64),
    Bool(bool),
    CodeItem(CodeItem),
    String(SharedStr),
}

impl ToTokens for VarValue {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            VarValue::CodeItem(s) => s.to_tokens(tokens),
            VarValue::String(s) => s.to_tokens(tokens),
            VarValue::Number(n) => n.to_tokens(tokens),
            VarValue::Bool(b) => b.to_tokens(tokens),
        }
    }
}

// *** Types ***

/// A hashmap of variables for interpolation into [CodeFragments]
pub type Vars = HashMap<SharedStr, VarItem>;

pub type CodeFragments = HashMap<SharedStr, &'static (dyn CodeFragment + Send + Sync)>;

/// A single code fragment - the smallest unit of work
pub trait CodeFragment {
    fn generate(&self, vars: &Vars) -> Result<TokenStream, CodeGenError>;
}
