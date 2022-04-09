# rust-format

[![Crate](https://img.shields.io/crates/v/rust-format)](https://crates.io/crates/rust-format)
[![Docs](https://docs.rs/rust-format/badge.svg)](https://docs.rs/rust-format)

A Rust source code formatting crate with a unified interface for string, file, and  
[TokenStream](https://docs.rs/proc-macro2/latest/proc_macro2/struct.TokenStream.html)
input. It currently supports [rustfmt](https://crates.io/crates/rustfmt-nightly) 
and [prettyplease](https://crates.io/crates/prettyplease). 

It optionally supports post-processing replacement of special blank/comment markers for 
inserting blank lines and comments in `TokenStream` generated source code 
respectively (as used by [quote-doctest](https://crates.io/crates/quote-doctest)
for inserting blanks/comments in generated doctests). It additionally supports
converting doc blocks (`#[doc =""]`) into doc comments (`///`). 

NOTE: This is primarily to support `rustfmt` as `prettyplease` automatically 
converts doc blocks into doc comments (but for `rustfmt` it requires nightly and
a configuration option).

## Usage

```toml
[dependencies]
rust-format = "0.3"
```

### Optional Features

* `post_process` - enables support for post-process conversion of special 
  "marker macros" into blank lines/comments. It additionally supports converting
  doc blocks (`#[doc]`) into doc comments (`///`)
* `pretty_please` - enables [prettyplease](https://crates.io/crates/prettyplease)
  formatting support
* `token_stream` - enables formatting from
  [TokenStream](https://docs.rs/proc-macro2/latest/proc_macro2/struct.TokenStream.html)
  input

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
  
  let mut config = Config::new_str()
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

`RustFmt` with post-processing:

```rust
use rust_format::{Config, Formatter, PostProcess, RustFmt};

fn main() {
  let source = r#"#[doc = " This is main"] fn main() { 
_blank_!(); _comment_!("\nThis prints hello world\n\n"); 
println!("Hello World!"); }"#;

  let mut config = Config::new_str()
      .post_proc(PostProcess::ReplaceMarkersAndDocBlocks);
  let actual = RustFmt::from_config(config).format_str(source).unwrap();
  let expected = r#"/// This is main
fn main() {

    //
    // This prints hello world
    //
    println!("Hello World!");
}
"#;

  assert_eq!(expected, actual);
}
```

## License

This project is licensed optionally under either:

* Apache License, Version 2.0, (LICENSE-APACHE
  or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license (LICENSE-MIT or https://opensource.org/licenses/MIT)
