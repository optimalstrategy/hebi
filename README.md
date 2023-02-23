<p align="center">
  <img
    alt="The greek lowercase letter mu inside of a gear shape"
    src="./assets/logo.svg"
    height="192px"
  >
</p>

# Mu

This repository hosts a dynamically typed language, its compiler, and VM.

Everything is still in flux, and many interesting features are not yet implemented (see [issues](https://github.com/jprochazk/mu/issues)), but it can already execute some basic code:

```rust
use mu::Mu;

let mu = Mu::new();

// prints `2`
println!("{}", mu.eval::<i32>("1 + 1").unwrap());

println!("{}", mu.eval::<()>(r#"
class Test:
  v = 10
  fn test(self):
    print self.v

t := Test(v=100)
t.test() # prints 100
t.v = 20
t.test() # prints 20
"#))
```

To see more examples, visit [src/tests](./src/tests).

The language also has a REPL at [examples/cli](./examples/cli):

```
$ cargo run --example cli
Mu REPL v0.0.0
Press CTRL-D to exit
> 
```

### Development

All you need to contribute is a recent version of the Rust compiler. See [Getting Started](https://www.rust-lang.org/learn/get-started) for how to obtain it.

Other tooling that is highly recommended:
- [rust-analyzer](https://rust-analyzer.github.io/), a Rust language server for your editor of choice
- [clippy](https://github.com/rust-lang/rust-clippy), a helpful linter
- [just](https://github.com/casey/just), which is used to run various commands


### Repository structure

- [`src`](./src) - The core crate, containing the runtime (bytecode compiler, register allocator, value representation, virtual machine).
  - [`op`](./src/op) - Bytecode instruction definitions, which define the fine-grained operations that the virtual machine may perform.
  - [`isolate`](./src/isolate) - The virtual machine, which implements the operations defined by `op`.
  - [`value`](./src/value) - Mu's Value implementation, which is how the virtual machine represents values at runtime.
  - [`emit`](./src/emit) - The bytecode compiler, which transforms an AST to executable bytecode.
  - [`tests`](./src/tests) - End-to-end tests of the full evaluation pipeline (`Code -> AST -> Bytecode -> Output`).
- [`crates`](./crates) - Parts of the compiler which may be useful for building other tools, such as formatters, linters, and language servers.
  - [`span`](./crates/span) - Span implementation. Spans are how the compiler represents locations in code.
  - [`syntax`](./crates/syntax) - The lexer, parser, and AST. This crate accepts some text input, and outputs a structured representation of valid code, or some syntax errors.
  - [`diag`](./crates/diag) - Diagnostic tools for reporting useful errors to users.
  - [`derive`](./crates/derive) - (WIP) Derive macros for easily exposing Rust functions and objects to the runtime.

### Design and implementation details

The language design is heavily inspired by Python. A general overview of the language's syntax is available in the [design](./design.md) file.

The VM borrows a lot of ideas from [V8](https://v8.dev/), namely the bytecode design which utilises an implicit accumulator to store temporary values, greatly reducing the number of frame sizes and register moves.

### License

Licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
