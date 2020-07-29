Framework for supporting different programming languages in TMC.

TMC-langs provides an interface that encapsulates everything needed to support a new language in TMC. The framework provides CLI wrappers so that it's fairly convenient to call from other languages like Ruby.

## Documentation

Documentation for the latest release is available at https://rage.github.io/tmc-langs-rust

## Building

Install Rust according to https://www.rust-lang.org/tools/install

```bash
git clone git@github.com:rage/tmc-langs-rust.git
cd tmc-langs-rust
cargo build
```

## Testing

```bash
cargo test
```

## Running the CLI

```bash
cargo run -p tmc-langs-cli help
```

## Development

Format using `cargo fmt`, use `cargo clippy` for linting.

## Deployment

Documentation and binaries for the supported targets are built and deployed to Google Cloud when creating a GitHub release. Each binary uses the release tag as a version number in the filename, for example `tmc-langs-cli-x86_64-unknown-linux-gnu-0.1.5-alpha` for the release `0.1.5-alpha`.

### Supported targets

- Linux 64-bit (x86_64-unknown-linux-gnu)
- Linux 32-bit (i686-unknown-linux-gnu)
- Windows MSVC 64-bit (x86_64-pc-windows-msvc)
- Windows MSVC 32-bit (i686-pc-windows-msvc)
- MacOS 64-bit (x86_64-apple-darwin)
- ARM64 (aarch64-unknown-linux-gnu)
- Armv7 (armv7-unknown-linux-gnueabihf)

## Included projects

### tmc-langs-cli

A binary CLI interface for TMC-langs for IDEs.

### tmc-langs-core

A library for communicating with the TMC server.

### tmc-langs-framework

A library for creating language plugins.

### tmc-langs-java

A TMC plugin for Java. Supports Maven and Ant.

### tmc-langs-make

A TMC plugin for Make.

### tmc-langs-notests

A TMC plugin for projects with no tests.

### tmc-langs-python3

A TMC plugin for Python 3.

### tmc-langs-r

A TMC plugin for R.

### tmc-langs-util

A library that provides a convenient interface abstracting over all available language plugins.
