use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

use flexstr::{local_fmt, LocalStr};
use proc_macro2::TokenStream;
use quote::{format_ident, ToTokens};

// *** Expand Vars ***

#[macro_export]
macro_rules! expand_vars {
    ($vars:ident, $($var:ident),+) => {
        $(let $var = {
            let var = flexstr::LocalStr::from_ref(&stringify!($var));
            let value = $vars.get(&var).ok_or($crate::CodeGenError::MissingVar(var))?;
            value
        };)+
    };
}

// *** CodeGenError ***

#[derive(Clone, Debug)]
pub enum CodeGenError {
    MissingVar(LocalStr),
}

impl Display for CodeGenError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CodeGenError::MissingVar(var) => {
                f.write_str(&local_fmt!("The specified variable '{var}' was missing."))
            }
        }
    }
}

impl Error for CodeGenError {}

// *** VarValue ***

#[derive(Clone, Debug)]
pub enum VarValue {
    Ident(LocalStr),
    String(LocalStr),
    Number(i64),
}

impl ToTokens for VarValue {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            VarValue::String(s) => s.to_tokens(tokens),
            VarValue::Number(n) => n.to_tokens(tokens),
            VarValue::Ident(s) => {
                let ident = format_ident!("{s}");
                ident.to_tokens(tokens);
            }
        }
    }
}

// *** CodeFragment

pub trait CodeFragment {
    fn generate(vars: &HashMap<LocalStr, VarValue>) -> Result<TokenStream, CodeGenError>;
}
