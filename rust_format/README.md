# rust-format

[![Crate](https://img.shields.io/crates/v/rust-format)](https://crates.io/crates/rust-format)
[![Docs](https://docs.rs/rust-format/badge.svg)](https://docs.rs/rust-format)

A Rust source code formatting crate with a unified interface for string, file, and  
[TokenStream](https://docs.rs/proc-macro2/latest/proc_macro2/struct.TokenStream.html)
input. It currently supports [rustfmt](https://crates.io/crates/rustfmt-nightly) 
and [prettyplease](https://crates.io/crates/prettyplease).

## Examples

Simple example using default options of `RustFmt`:

```rust
use rust_format::{Formatter, RustFmt};

fn main() {
  let source = r#"fn main() { println!("Hello World!"); }"#;

  let actual = RustFmt::default().format_str(source).unwrap();
  let expected = r#"fn main() {
    println!("Hello World!");
}
"#;

  assert_eq!(expected, actual);
}
```

Using a custom configuration:

```rust
use rust_format::{Config, Edition, Formatter, RustFmt};

fn main() {
  let source = r#"use std::marker; use std::io; mod test; mod impls;"#;
  
  let mut config = Config::new_basic()
    .edition(Edition::Rust2018)
    .option("reorder_imports", "false")
    .option("reorder_modules", "false");
  let rustfmt = RustFmt::from_config(config);
  
  let actual = rustfmt.format_str(source).unwrap();
  let expected = r#"use std::marker;
use std::io;
mod test;
mod impls;
"#;
  
  assert_eq!(expected, actual);
}
```

## Install

```toml
[dependencies]
rust-format = "0.2"
```

### Optional Features

* `pretty_please` - enables [prettyplease](https://crates.io/crates/prettyplease)
  formatting support
* `token_stream` - enables formatting from 
  [TokenStream](https://docs.rs/proc-macro2/latest/proc_macro2/struct.TokenStream.html)
  input

## License

This project is licensed optionally under either:

* Apache License, Version 2.0, (LICENSE-APACHE
  or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license (LICENSE-MIT or https://opensource.org/licenses/MIT)
