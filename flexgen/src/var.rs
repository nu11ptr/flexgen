use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use flexstr::{shared_str, SharedStr, ToSharedStr};
use proc_macro2::TokenStream;
use quote::ToTokens;

use crate::Error;

const IDENT: &str = "$ident$";
const INT_LIT: &str = "$int_lit$";
const TYPE: &str = "$type$";

/// A hashmap of variables for interpolation into [CodeFragments]
pub(crate) type Vars = HashMap<SharedStr, VarItem>;

/// Represents a map of variables ready for interpolation
pub type TokenVars = HashMap<SharedStr, TokenItem>;

// *** Expand Vars ***

#[doc(hidden)]
#[inline]
pub fn import_var<'vars>(
    vars: &'vars TokenVars,
    var: &'static str,
) -> Result<&'vars TokenValue, Error> {
    let var = shared_str!(var);
    let value = vars.get(&var).ok_or(Error::MissingVar(var))?;

    match value {
        TokenItem::Single(value) => Ok(value),
        TokenItem::List(_) => Err(Error::WrongItem),
    }
}

/// Import the variables from the [Config](crate::config::Config) into local variables that can be interpolated with `quote`
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
) -> Result<&'vars [TokenValue], Error> {
    let var = shared_str!(var);
    let value = vars.get(&var).ok_or(Error::MissingVar(var))?;

    match value {
        TokenItem::List(value) => Ok(value),
        TokenItem::Single(_) => Err(Error::WrongItem),
    }
}

/// Import the list of variables from the [Config](crate::config::Config) into local bindings that can be interpolated with `quote`
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

#[inline]
fn strip_prefix(s: &str, prefix: &str) -> Option<SharedStr> {
    if matches!(s.find(prefix), Some(idx) if idx == 0) {
        Some(s[prefix.len()..].to_shared_str())
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CodeValue {
    Ident(SharedStr),
    IntLit(SharedStr),
    Type(SharedStr),
}

impl FromStr for CodeValue {
    type Err = Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(s) = strip_prefix(s, IDENT) {
            Ok(CodeValue::Ident(s))
        } else if let Some(s) = strip_prefix(s, INT_LIT) {
            Ok(CodeValue::IntLit(s))
        } else if let Some(s) = strip_prefix(s, TYPE) {
            Ok(CodeValue::Type(s))
        } else {
            Err(Error::NotCodeItem(s.to_shared_str()))
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

/// A single code-related token variable from the [Config](crate::config::Config)
#[derive(Clone, Debug, PartialEq)]
pub enum CodeTokenValue {
    /// An identifier
    Ident(syn::Ident),
    /// An integer literal
    IntLit(syn::LitInt),
    /// A type
    Type(Box<syn::Type>),
}

impl CodeTokenValue {
    #[inline]
    pub(crate) fn new(item: &CodeValue) -> Result<Self, Error> {
        match item {
            CodeValue::Ident(i) => Ok(CodeTokenValue::Ident(syn::parse_str::<syn::Ident>(i)?)),
            CodeValue::IntLit(i) => Ok(CodeTokenValue::IntLit(syn::parse_str::<syn::LitInt>(i)?)),
            CodeValue::Type(t) => Ok(CodeTokenValue::Type(Box::new(syn::parse_str::<syn::Type>(
                t,
            )?))),
        }
    }
}

impl fmt::Display for CodeTokenValue {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeTokenValue::Ident(i) => <syn::Ident as fmt::Display>::fmt(i, f),
            CodeTokenValue::IntLit(i) => <syn::LitInt as fmt::Display>::fmt(i, f),
            CodeTokenValue::Type(t) => <syn::Type as fmt::Debug>::fmt(t, f),
        }
    }
}

impl ToTokens for CodeTokenValue {
    #[inline]
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            CodeTokenValue::Ident(ident) => ident.to_tokens(tokens),
            CodeTokenValue::IntLit(lit) => lit.to_tokens(tokens),
            CodeTokenValue::Type(t) => t.to_tokens(tokens),
        }
    }
}

// *** VarItem ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum VarItem {
    List(Vec<VarValue>),
    Single(VarValue),
}

impl VarItem {
    #[inline]
    pub fn to_token_item(&self) -> Result<TokenItem, Error> {
        match self {
            VarItem::List(l) => {
                let items: Vec<_> = l
                    .iter()
                    .map(|item| item.to_token_value())
                    .collect::<Result<Vec<TokenValue>, Error>>()?;
                Ok(TokenItem::List(items))
            }
            VarItem::Single(s) => Ok(TokenItem::Single(s.to_token_value()?)),
        }
    }
}

// *** VarValue ***

#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(untagged)]
pub(crate) enum VarValue {
    Number(i64),
    Bool(bool),
    CodeValue(CodeValue),
    String(SharedStr),
}

impl VarValue {
    #[inline]
    fn to_token_value(&self) -> Result<TokenValue, Error> {
        Ok(match self {
            VarValue::Number(n) => TokenValue::Number(*n),
            VarValue::Bool(b) => TokenValue::Bool(*b),
            VarValue::CodeValue(c) => TokenValue::CodeValue(CodeTokenValue::new(c)?),
            VarValue::String(s) => TokenValue::String(s.clone()),
        })
    }
}

// *** TokenItem ***

/// Represents either a list of variables or a single variable from the [Config](crate::config::Config)
#[derive(Clone, Debug, PartialEq)]
pub enum TokenItem {
    /// A list of values
    List(Vec<TokenValue>),
    /// A single value
    Single(TokenValue),
}

// *** TokenValue ***

/// A single variable from the [Config](crate::config::Config)
#[derive(Clone, Debug, PartialEq)]
pub enum TokenValue {
    /// A numeric value
    Number(i64),
    /// A boolean value
    Bool(bool),
    /// A code token value
    CodeValue(CodeTokenValue),
    /// A string value
    String(SharedStr),
}

impl fmt::Display for TokenValue {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenValue::Number(n) => <i64 as fmt::Display>::fmt(n, f),
            TokenValue::Bool(b) => <bool as fmt::Display>::fmt(b, f),
            TokenValue::CodeValue(c) => <CodeTokenValue as fmt::Display>::fmt(c, f),
            TokenValue::String(s) => <SharedStr as fmt::Display>::fmt(s, f),
        }
    }
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

#[cfg(test)]
mod tests {
    use crate::var::CodeValue;
    use flexstr::shared_str;
    use std::str::FromStr;

    #[test]
    fn code_value_from_str() {
        assert_eq!(
            CodeValue::from_str("$type$str").unwrap(),
            CodeValue::Type(shared_str!("str"))
        );
        assert_eq!(
            CodeValue::from_str("$ident$str").unwrap(),
            CodeValue::Ident(shared_str!("str"))
        );
        assert_eq!(
            CodeValue::from_str("$int_lit$123").unwrap(),
            CodeValue::IntLit(shared_str!("123"))
        );
    }
}
