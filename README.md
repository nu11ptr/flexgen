# flexgen

[![Crate](https://img.shields.io/crates/v/flexgen)](https://crates.io/crates/flexgen)
[![Docs](https://docs.rs/flexgen/badge.svg)](https://docs.rs/flexgen)

A flexible, yet simple quote-based code generator for creating beautiful Rust code

## Why?

Rust has two types of macros, and they are both very popular, however, they
are not always the optimal choice. They can impact build performance and make
the source code more obfuscated to read and study. Regular macros make it difficult
to do much more than simple variable substitution and using `quote` via proc-macro
doesn't allow variable interpolation in doc blocks (see
[quote-doctest](https://crates.io/crates/quote-doctest) for a solution).

Code generation isn't perfect either. It creates excess code which is
likely to be highly duplicated and thus create "noise". However, it can
also be nice to have a complete set of source code available and easily
reachable via the docs. Since we can generate it ahead of time, its impact
on performance is the same as regular Rust code.

The right solution likely depends on the use case. I personally think macros
tend to be better for writing either very simple duplication or very fancy
things that are hard or impossible without them. Code generation is more niche
but works well for generating bulk wrapper code esp. for code that is slightly
different per type and requires more logic to handle (esp. in doctests).

## Example

It is probably easiest to look at the "fibonacci" example: 
[directory](https://github.com/nu11ptr/flexgen/tree/master/flexgen/examples/basic)

* `fib.rs` - the generated file
* `flexgen.toml` - the configuration file
* `main.rs` - the source file that generates `fib.rs`

To run yourself:

1. Change into the `examples/basic` directory
2. Delete the existing `fib.rs` file
3. Run: `cargo run --example basic`
4. Compile the new fib.rs file: `rustc fib.rs -C opt-level=3`
5. Run it: `./fib`

## Usage

1. Create a new binary crate (`flexgen` is a library, not a binary crate)

2. Edit `Cargo.toml` with any needed dependencies (at minimum, `flexgen`, but 
 you will likely want `quote` and possibly `quote-doctest` as well)

```toml
[dependencies]
flexgen = "0.4"
```

3. Edit your `main.rs` and add in one or more code fragments implementing 
`CodeFragment`. How much code a fragment contains is a process of trial and error,
but typically it would be "one thing" (ie. one function). See the example above 
for more details.

```rust
// main.rs

use flexgen::var::TokenVars;
use flexgen::{import_vars, CodeFragment, Error};
use quote::quote;

struct HelloWorld;

impl CodeFragment for HelloWorld {
    fn generate(&self, vars: &TokenVars) -> Result<TokenStream, Error> {
        import_vars! { vars => hello };

        Ok(quote! {
            fn main() {
                println!("{hello} world!");
            }
        })
    }
}
```

4. Create and edit `flexgen.toml`

NOTE: All the possible options can be found in the test code 
[here](https://github.com/nu11ptr/flexgen/blob/68de04679ce568981c72fdde1db8f8987332964f/flexgen/src/config.rs#L316)

```toml
# flexgen.toml

[fragment_lists]
hello = [ "hello_world" ]

[files.hello]
path = "hello.rs"
fragment_list = "hello"

[files.hello.vars]
hello = "Hello"
```

5. Add a `main` function to your `main.rs` file

```rust
// main.rs

use flexgen::config::Config;
use flexgen::{register_fragments, Error, CodeGenerator};

fn main() -> Result<(), Error> {
    // Register all your code fragments
    let fragments = register_fragments!(HelloWorld);
    // Read in the configuration from our flexgen.toml file
    let config = Config::from_default_toml_file()?;
    // Create a new code generator from our fragments and config
    let gen = CodeGenerator::new(fragments, config)?;
    // Generate our 'hello.rs' file
    gen.generate_files()
}
```

6. Execute your binary to generate the code

```
cargo run
```

## License

This project is licensed optionally under either:

* Apache License, Version 2.0, (LICENSE-APACHE
  or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license (LICENSE-MIT or https://opensource.org/licenses/MIT)
