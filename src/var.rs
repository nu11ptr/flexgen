use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use flexstr::{shared_str, SharedStr, ToSharedStr};
use proc_macro2::TokenStream;
use quote::ToTokens;

use crate::CodeGenError;

const IDENT: &str = "$ident$";

/// A hashmap of variables for interpolation into [CodeFragments]
pub type Vars = HashMap<SharedStr, VarItem>;

pub type TokenVars = HashMap<SharedStr, TokenItem>;

// *** Expand Vars ***

#[doc(hidden)]
#[inline]
pub fn import_var<'vars>(
    vars: &'vars TokenVars,
    var: &'static str,
) -> Result<&'vars TokenValue, CodeGenError> {
    let var = shared_str!(var);
    let value = vars.get(&var).ok_or(CodeGenError::MissingVar(var))?;

    match value {
        TokenItem::Single(value) => Ok(value),
        TokenItem::List(_) => Err(CodeGenError::WrongItem),
    }
}

#[macro_export]
macro_rules! import_vars {
    // Allow trailing comma
    ($vars:ident => $($var:ident,)+) => { $crate::var::import_vars!($vars, $($var),+) };
    ($vars:ident => $($var:ident),+) => {
        $(
            let $var = $crate::var::import_var($vars, stringify!($var))?;
        )+
    };
}

#[doc(hidden)]
#[inline]
pub fn import_list<'vars>(
    vars: &'vars TokenVars,
    var: &'static str,
) -> Result<&'vars [TokenValue], CodeGenError> {
    let var = shared_str!(var);
    let value = vars.get(&var).ok_or(CodeGenError::MissingVar(var))?;

    match value {
        TokenItem::List(value) => Ok(value),
        TokenItem::Single(_) => Err(CodeGenError::WrongItem),
    }
}

#[macro_export]
macro_rules! import_lists {
    // Allow trailing comma
    ($vars:ident => $($var:ident,)+) => { $crate::var::import_lists!($vars, $($var),+) };
    ($vars:ident => $($var:ident),+) => {
        $(
            let $var = $crate::var::import_list($vars, stringify!($var))?;
        )+
    };
}

// *** CodeValue ***

#[derive(Clone, Debug, PartialEq)]
pub enum CodeValue {
    Ident(SharedStr),
}

impl FromStr for CodeValue {
    type Err = CodeGenError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if matches!(s.find(IDENT), Some(idx) if idx == 0) {
            Ok(CodeValue::Ident(s[IDENT.len()..].to_shared_str()))
        } else {
            Err(CodeGenError::NotCodeItem(s.to_shared_str()))
        }
    }
}

struct SynItemVisitor;

impl<'de> serde::de::Visitor<'de> for SynItemVisitor {
    type Value = CodeValue;

    #[inline]
    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string with a special prefix")
    }

    #[inline]
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse()
            .map_err(|_| serde::de::Error::custom("Error deserializing 'str'"))
    }

    #[inline]
    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        v.parse()
            .map_err(|_| serde::de::Error::custom("Error deserializing 'String'"))
    }
}

impl<'de> serde::de::Deserialize<'de> for CodeValue {
    #[inline]
    fn deserialize<D: serde::de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(SynItemVisitor)
    }
}

// *** CodeTokenValue ***

#[derive(Clone, Debug, PartialEq)]
pub enum CodeTokenValue {
    Ident(syn::Ident),
}

impl CodeTokenValue {
    #[inline]
    pub fn new(item: &CodeValue) -> Result<Self, CodeGenError> {
        match item {
            CodeValue::Ident(i) => Ok(CodeTokenValue::Ident(syn::parse_str::<syn::Ident>(i)?)),
        }
    }
}

impl ToTokens for CodeTokenValue {
    #[inline]
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            CodeTokenValue::Ident(ident) => ident.to_tokens(tokens),
        }
    }
}

// *** VarItem ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum VarItem {
    List(Vec<VarValue>),
    Single(VarValue),
}

impl VarItem {
    #[inline]
    pub fn to_token_item(&self) -> Result<TokenItem, CodeGenError> {
        match self {
            VarItem::List(l) => {
                let items: Vec<_> = l
                    .iter()
                    .map(|item| item.to_token_value())
                    .collect::<Result<Vec<TokenValue>, CodeGenError>>()?;
                Ok(TokenItem::List(items))
            }
            VarItem::Single(s) => Ok(TokenItem::Single(s.to_token_value()?)),
        }
    }
}

// *** VarValue ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub enum VarValue {
    Number(i64),
    Bool(bool),
    CodeValue(CodeValue),
    String(SharedStr),
}

impl VarValue {
    #[inline]
    fn to_token_value(&self) -> Result<TokenValue, CodeGenError> {
        Ok(match self {
            VarValue::Number(n) => TokenValue::Number(*n),
            VarValue::Bool(b) => TokenValue::Bool(*b),
            VarValue::CodeValue(c) => TokenValue::CodeValue(CodeTokenValue::new(c)?),
            VarValue::String(s) => TokenValue::String(s.clone()),
        })
    }
}

// *** TokenItem ***

#[derive(Clone, Debug, PartialEq)]
pub enum TokenItem {
    List(Vec<TokenValue>),
    Single(TokenValue),
}

// *** TokenValue ***

#[derive(Clone, Debug, PartialEq)]
pub enum TokenValue {
    Number(i64),
    Bool(bool),
    CodeValue(CodeTokenValue),
    String(SharedStr),
}

impl ToTokens for TokenValue {
    #[inline]
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            TokenValue::CodeValue(c) => c.to_tokens(tokens),
            TokenValue::String(s) => s.to_tokens(tokens),
            TokenValue::Number(n) => n.to_tokens(tokens),
            TokenValue::Bool(b) => b.to_tokens(tokens),
        }
    }
}
