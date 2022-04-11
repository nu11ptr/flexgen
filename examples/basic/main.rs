use flexgen::config::Config;
use flexgen::var::TokenVars;
use flexgen::{import_vars, register_fragments, CodeFragment, CodeGenError, CodeGenerator};
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

fn main() -> Result<(), CodeGenError> {
    let fragments = register_fragments!(Function);
    let config = Config::from_default_toml_file()?;
    let executor = CodeGenerator::new(fragments, config)?;
    executor.generate_files()
}
