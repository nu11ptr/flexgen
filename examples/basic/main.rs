use flexgen::config::Config;
use flexgen::var::TokenVars;
use flexgen::{import_vars, register_fragments, CodeFragment, CodeGenError, CodeGenerator};
use proc_macro2::TokenStream;
use quote::quote;
use quote_doctest::doc_test;

struct DocTest;

impl CodeFragment for DocTest {
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
        import_vars! { vars => fib, one };

        let test = quote! {
            assert_eq!(#fib(10), 55);
            assert_eq!(#fib(#one), #one);
        };

        Ok(doc_test!(test)?)
    }
}

struct Function;

impl CodeFragment for Function {
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
        import_vars! { vars => fib, one };

        let doc_test = DocTest.generate(vars)?;

        Ok(quote! {
            /// This will run a compare between fib inputs and the outputs
            #doc_test
            #[inline]
            fn #fib(n: u64) -> u64 {
                match n {
                    0 => 0,
                    #one => #one,
                    n => #fib(n - 1) + #fib(n - 2),
                }
            }
        })
    }
}

struct Main;

impl CodeFragment for Main {
    fn uses(&self) -> Option<TokenStream> {
        Some(quote! {
            use std::error::{Error as StdError};
            use std::io::stdin;
        })
    }

    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, CodeGenError> {
        import_vars! { vars => fib };

        Ok(quote! {
            /// This is the main function
            fn main() -> Result<(), Box<dyn StdError>>  {
                println!("Enter a number:");
                let mut line = String::new();
                stdin().read_line(&mut line)?;
                let num: u64 = line.trim_end().parse()?;
                _blank_!();

                _comment_!("\nCalculate fibonacci for user input\n\n");
                let answer = #fib(num);
                println!("The number '{num}' in the fibonacci sequence is: {answer}");
                _blank_!();

                Ok(())
            }
        })
    }
}

fn main() -> Result<(), CodeGenError> {
    let fragments = register_fragments!(Function, Main);
    let config = Config::from_default_toml_file()?;
    let gen = CodeGenerator::new(fragments, config)?;
    gen.generate_files()
}
