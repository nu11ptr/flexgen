# use-builder

[![Crate](https://img.shields.io/crates/v/use-builder)](https://crates.io/crates/use-builder)
[![Docs](https://docs.rs/use-builder/badge.svg)](https://docs.rs/use-builder)

A crate to build source code use sections by combining multiple (possibly duplicate)
use section inputs.

NOTE: This is a fairly specialized crate. The only likely use case is really that
of compiling source code snippets into files, like flexgen does.

## Usage

```toml
[dependencies]
use-builder = "0.1"
```

## Example

```rust
use assert_unordered::assert_eq_unordered;
use quote::quote;
use use_builder::{UseBuilder, UseItems};

fn main() {
    // #1 - Build a two or more use trees and convert into `UseItems` (wrapped `Vec<ItemUse>`)

    let use1 = quote! {
        use crate::Test;
        use std::error::{Error as StdError};
        use std::fmt::Debug;
    };

    let use2 = quote! {
        use syn::ItemUse;
        use std::fmt::Display;
        use crate::*;
    };

    let items1: UseItems = syn::parse2(use1).unwrap();
    let items2: UseItems = syn::parse2(use2).unwrap();

    // #2 - Parse, process, and extract into sections

    let builder = UseBuilder::from_uses(vec![items1, items2]);
    let (std_use, ext_use, crate_use) = builder.into_items_sections().unwrap();

    // #3 - Validate our response matches expectation

    let std_expected = quote! {
        use std::error::Error as StdError;
        use std::fmt::{Debug, Display};
    };
    let std_expected = syn::parse2::<UseItems>(std_expected).unwrap().into_inner();

    let ext_expected = quote! {
        use syn::ItemUse;
    };
    let ext_expected = syn::parse2::<UseItems>(ext_expected).unwrap().into_inner();

    let crate_expected = quote! {
        use crate::*;
    };
    let crate_expected = syn::parse2::<UseItems>(crate_expected).unwrap().into_inner();

    assert_eq_unordered!(std_expected, std_use);
    assert_eq_unordered!(ext_expected, ext_use);
    assert_eq_unordered!(crate_expected, crate_use);
}
```

## License

This project is licensed optionally under either:

* Apache License, Version 2.0, (LICENSE-APACHE
  or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license (LICENSE-MIT or https://opensource.org/licenses/MIT)
