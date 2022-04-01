# quote-doctest
A simple doctest generator for [quote](https://github.com/dtolnay/quote)

## Overview

Currently, quote 
[does not support](https://docs.rs/quote/latest/quote/macro.quote.html#interpolating-text-inside-of-doc-comments) 
interpolation inside of comments, which means no doctests. This crate 
provides a simple mechanism to generate doctests for inclusion in generated code.

```toml
[dependencies]
quote-doctest = "0.1"
```

## Example

```rust
use quote::quote;

fn main() {
    // Takes any `TokenStream` as input (but typically `quote` would be used)
    let doc_test = quote_doctest::doc_test!(quote! {
        assert_eq!(fibonacci(10), 55);
        assert_eq!(fibonacci(1), 1);
    }).unwrap();

    // Interpolates into a regular `quote` invocation
    let actual = quote! {
        /// This will run a compare between fib inputs and the outputs
        #doc_test
        fn fibonacci(n: u64) -> u64 {
            match n {
                0 => 1,
                1 => 1,
                n => fibonacci(n - 1) + fibonacci(n - 2),
            }
        }
    };

    // This is what is generated:
    let expected = quote! {
        #[doc = r" This will run a compare between fib inputs and the outputs"]
        #[doc = "```"]
        #[doc = "assert_eq!(fibonacci(10), 55);"]
        #[doc = "assert_eq!(fibonacci(1), 1);"]
        #[doc = "```"]
        fn fibonacci(n: u64) -> u64 {
            match n {
                0 => 1,
                1 => 1,
                n => fibonacci(n - 1) + fibonacci(n - 2),
            }
        }
    };
    
    assert_eq!(expected.to_string(), actual.to_string());
}
```

## Notes
- If generating source code (instead of using in a macro), be aware that 
  `TokenStream` will render in `#[doc]` attribute format, not as a `///` comment
  - `rustfmt` nightly has an option to translate these
- By default, this calls out to `rustfmt` in order to translate this into a 
  list of lines. Omitting formatting is possible, but the resulting 
  doctest will be a single `#[doc]` attribute (which will result in poor 
  looking rustdoc) 

## License

This project is licensed optionally under either:

* Apache License, Version 2.0, (LICENSE-APACHE
  or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license (LICENSE-MIT or https://opensource.org/licenses/MIT)
