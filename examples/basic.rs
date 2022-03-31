use std::collections::HashMap;

use flexgen::{doc_test, expand_vars, CodeFragment, CodeGenError, VarValue};
use flexstr::{local_str, LocalStr};
use proc_macro2::TokenStream;
use quote::quote;

struct DocTest;

impl CodeFragment for DocTest {
    fn generate(vars: &HashMap<LocalStr, VarValue>) -> Result<TokenStream, CodeGenError> {
        expand_vars!(vars, fib);

        let test = quote! {
            assert_eq!(#fib(10), 55);
            assert_eq!(#fib(1), 1);
        };

        let doc_test = doc_test!(test).unwrap();
        Ok(doc_test)
    }
}

struct Function;

impl CodeFragment for Function {
    fn generate(vars: &HashMap<LocalStr, VarValue>) -> Result<TokenStream, CodeGenError> {
        expand_vars!(vars, fib);

        let doc_test = DocTest::generate(vars)?;

        Ok(quote! {
            #doc_test
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
