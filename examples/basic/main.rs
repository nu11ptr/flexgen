use std::collections::HashMap;

use flexgen::{
    import_vars, register_fragments, CodeFragment, CodeGenError, VarItem, VarValue, Vars,
};
use flexstr::shared_str;
use proc_macro2::TokenStream;
use quote::quote;
use quote_doctest::doc_test;

struct DocTest;

impl CodeFragment for DocTest {
    fn generate(&self, vars: &Vars) -> Result<TokenStream, CodeGenError> {
        import_vars!(vars => fib);

        let test = quote! {
            assert_eq!(#fib(10), 55);
            assert_eq!(#fib(1), 1);
            println!("Fib: {}", #fib(12));
        };

        let doc_test = doc_test!(test).unwrap();
        Ok(doc_test)
    }
}

struct Function;

impl CodeFragment for Function {
    fn generate(&self, vars: &Vars) -> Result<TokenStream, CodeGenError> {
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
        VarItem::Single(VarValue::CodeItem("$ident$fibonacci".parse().unwrap())),
    );

    let fib = Function.generate(&map).unwrap().to_string();
    println!("{}", fib);
}
