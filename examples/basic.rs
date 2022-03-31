use std::collections::HashMap;

use flexgen::{expand_vars, CodeFragment, CodeGenError, VarValue};
use flexstr::{local_str, LocalStr};
use proc_macro2::TokenStream;
use quote::quote;

struct Function;

impl CodeFragment for Function {
    fn generate(vars: &HashMap<LocalStr, VarValue>) -> Result<TokenStream, CodeGenError> {
        expand_vars!(vars, fib);

        Ok(quote! {
            #[inline]
            fn #fib(n: u64) -> u64 {
                match n {
                    0 => 1,
                    1 => 1,
                    n => #fib(n - 1) + #fib(n - 2),
                }
            }
        })
    }
}

fn main() {
    let mut map = HashMap::new();
    map.insert(local_str!("fib"), VarValue::Ident(local_str!("fibonacci")));

    let fib = Function::generate(&map).unwrap().to_string();
    println!("{}", fib);
}
