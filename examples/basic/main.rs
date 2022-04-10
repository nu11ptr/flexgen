use std::collections::HashMap;

use flexgen::var::{CodeTokenValue, CodeValue, TokenItem, TokenValue, TokenVars};
use flexgen::{import_vars, register_fragments, CodeFragment, CodeGenError};
use flexstr::shared_str;
use proc_macro2::TokenStream;
use quote::quote;
use quote_doctest::doc_test;

struct DocTest;

impl CodeFragment for DocTest {
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
        import_vars!(vars => fib);

        let test = quote! {
            assert_eq!(#fib(10), 55);
            assert_eq!(#fib(1), 1);
            println!("Fib: {}", #fib(12));
        };

        Ok(doc_test!(test)?)
    }
}

struct Function;

impl CodeFragment for Function {
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
        import_vars!(vars => fib);

        let doc_test = DocTest.generate(vars)?;

        Ok(quote! {
            /// This will run a compare between fib inputs and the outputs
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
    let _fragments = register_fragments!(Function);

    let mut map = HashMap::new();
    map.insert(
        shared_str!("fib"),
        TokenItem::Single(TokenValue::CodeValue(
            CodeTokenValue::new(&CodeValue::Ident(shared_str!("fibonacci"))).unwrap(),
        )),
    );

    let fib = Function.generate(&map).unwrap().to_string();
    println!("{}", fib);
}
