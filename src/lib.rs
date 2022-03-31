use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::anyhow;
use flexstr::{local_fmt, LocalStr};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};

const RUST_FMT: &str = "rustfmt";
const RUST_FMT_KEY: &str = "RUSTFMT";
const CODE_MARKER: &str = "```";

#[macro_export]
macro_rules! doc_test {
    ($tokens:expr) => {
        $crate::make_doc_test($tokens, true, true, 4);
    };
    ($tokens:expr, $fmt:expr) => {
        // No point in generating main if not sending to 'rustfmt'
        $crate::make_doc_test($tokens, $fmt, false, 4);
    };
    ($tokens:expr, $fmt:expr, $gen_main:expr) => {
        $crate::make_doc_test($tokens, $fmt, $gen_main, 4);
    };
    ($tokens:expr, $fmt:expr, $gen_main:expr, $strip_indent:expr) => {
        $crate::make_doc_test($tokens, $fmt, $gen_main, $strip_indent);
    };
}

pub fn make_doc_test(
    mut tokens: TokenStream,
    fmt: bool,
    gen_main: bool,
    strip_indent: usize,
) -> anyhow::Result<TokenStream> {
    // No point generating 'main' without formatting
    let gen_main = gen_main && fmt;
    // No point stripping indent (from rustfmt due to adding 'main') if we aren't generating main
    let strip_indent = if gen_main { strip_indent } else { 0 };

    // Surround with main, if needed (and we can't remove it unless we are formatting)
    if gen_main {
        tokens = quote! {
            fn main() { #tokens }
        };
    }

    // Convert to string source code format
    let mut src = tokens.to_string();

    // Format it, if requested
    if fmt {
        let (source, _) = format_rs_str(&src)?;
        src = source;
    }

    // Split string source code into lines
    let lines = src.lines();
    let cap = lines.size_hint().0 + 2;

    // Remove `fn main () {`, if we added it
    let lines = if gen_main {
        lines.skip(1)
    } else {
        lines.skip(0)
    };
    let mut doc_test = Vec::with_capacity(cap);
    let prefix = " ".repeat(strip_indent);

    doc_test.push(CODE_MARKER);
    for mut line in lines {
        // Strip whitespace left over from main
        line = line.strip_prefix(&prefix).unwrap_or(line);
        doc_test.push(line);
    }
    // Remove the trailing `}`
    if gen_main {
        doc_test.pop();
    }
    doc_test.push(CODE_MARKER);

    // Convert into doc attributes
    Ok(quote! {
        #( #[doc = #doc_test] )*
    })
}

fn format_rs_str(s: impl AsRef<str>) -> anyhow::Result<(String, String)> {
    // Use 'rustfmt' specified by the environment var, if specified, else use the default
    let rustfmt = env::var(RUST_FMT_KEY).unwrap_or_else(|_| RUST_FMT.to_string());

    // Launch rustfmt
    let mut proc = Command::new(&rustfmt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Get stdin and send our source code to it to be formatted
    // Safety: Can't panic - we captured stdin above
    let mut stdin = proc.stdin.take().unwrap();
    eprintln!("{}", s.as_ref());
    stdin.write_all(s.as_ref().as_bytes())?;
    // Close stdin
    drop(stdin);

    // Parse the results and return stdout/stderr
    let output = proc.wait_with_output()?;
    if output.status.success() {
        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;
        Ok((stdout, stderr))
    } else {
        Err(anyhow!(
            "An error occurred while attempting to execute: '{rustfmt}'"
        ))
    }
}

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
