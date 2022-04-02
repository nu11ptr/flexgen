# quote-doctest

[![Crate](https://img.shields.io/crates/v/quote-doctest)](https://crates.io/crates/quote-doctest)
[![Docs](https://docs.rs/quote-doctest/badge.svg)](https://docs.rs/quote-doctest)

A simple doctest and doc comment generator for [quote](https://crates.io/crates/quote)

## Overview

Currently, quote 
[does not support](https://docs.rs/quote/latest/quote/macro.quote.html#interpolating-text-inside-of-doc-comments) 
interpolation inside of comments, which means no customized doctests. This 
crate provides a simple mechanism to generate doctests and doc comments for 
inclusion in generated code.

```toml
[dependencies]
quote-doctest = "0.2"
```

## Example

Using the `doc_test` macro, we can take any `TokenStream` and turn it into
a doctest `TokenStream` that can be interpolated in any `quote` macro 
invocation. 

The `doc_comment` function takes any string and turns it into one or more 
comments inside a `TokenStream`.

```rust
use quote::quote;
use quote_doctest::{doc_comment, doc_test};

fn main() {
    // Takes any `TokenStream` as input (but typically `quote` would be used)
    let test = doc_test!(quote! {
        _comment!("Calling fibonacci with 10 returns 55");
        assert_eq!(fibonacci(10), 55);
    
        _blank!();
        _comment!("Calling fibonacci with 1 simply returns 1");
        assert_eq!(fibonacci(1), 1);
    }).unwrap();
  
    let comment = doc_comment("This compares between fib inputs and outputs:\n\n").unwrap();
  
    // Interpolates into a regular `quote` invocation
    let actual = quote! {
        #comment
        #test
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
        /// This compares between fib inputs and outputs:
        ///
        /// ```
        /// // Calling fibonacci with 10 returns 55
        /// assert_eq!(fibonacci(10), 55);
        ///
        /// // Calling fibonacci with 1 simply returns 1
        /// assert_eq!(fibonacci(1), 1);
        /// ```
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
- It can use both [prettyplease](https://crates.io/crates/prettyplease) 
  (default) or the system `rustfmt` for formatting the doctests
    - It honors the `RUSTFMT` environment variable if set (and using `rustfmt`)
- Since comments and blank lines are whitespace to the parser, marker macros 
  are used to map out where the comments and blank lines should appear. 
  These will be replaced by comments and blank lines respectively in the 
  doctest (as shown in the example above)

## License

This project is licensed optionally under either:

* Apache License, Version 2.0, (LICENSE-APACHE
  or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license (LICENSE-MIT or https://opensource.org/licenses/MIT)
