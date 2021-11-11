## Building

Install Rust according to https://www.rust-lang.org/tools/install

Install [zstd 1.4.9](https://github.com/facebook/zstd). For example, on Ubuntu you need the package `libzstd1`. For Windows, download the appropriate archive from the [releases](https://github.com/facebook/zstd/releases), extract it and add the extracted directory to your PATH.

Install [Node.js and npm](https://docs.npmjs.com/downloading-and-installing-node-js-and-npm).

Install [Python 3](https://www.python.org/downloads/). You may wish to use [pyenv](https://github.com/pyenv/pyenv/) to manage different Python versions conveniently.

Install [OpenJDK 11](https://openjdk.java.net/install/index.html) (other versions may work as well) and [ant](https://ant.apache.org/).

Install [.NET 5.0](https://dotnet.microsoft.com/download)

Install [check](https://libcheck.github.io/check/) (works with at least 0.14 and 0.15)

Install [R](https://www.r-project.org/), [devtools](https://devtools.r-lib.org/) with `install.packages("devtools")` and [tmc-r-tester](https://github.com/testmycode/tmc-rstudio) with `devtools::install_github("testmycode/tmc-r-tester/tmcRtestrunner", build = FALSE)`

```bash
git clone git@github.com:rage/tmc-langs-rust.git
cd tmc-langs-rust
cargo build
```

If you have any troubles building the project, please do make an issue!

## Testing

```bash
cargo test
```

## Formatting and linting

Use `cargo fmt` and `cargo clippy` for formatting and linting. All crates should have the clippy lints `print_stdout` and `print_stderr` set to deny to allow the CLI to have total control over stdout and stderr. The CLI has one function where writing to stdout is allowed.

## Updating dependencies

For convenience, a tool called [cargo-outdated](https://crates.io/crates/cargo-outdated) can be installed to automatically check all the crates in the workspace for outdated dependencies. You may want to call `cargo update` first to update dependencies to the latest semver-compatible version.

In addition to the dependencies listed in each crate's `Cargo.toml`, the project bundles a few external dependencies such as `tmc-checkstyle-runner`, `tmc-junit-runner` and so on. When updating dependencies, you may want to check whether these projects have been updated.

## Versioning

tmc-langs-rust follows Rust-style semantic versioning, but only for the `tmc-langs-cli` and `tmc-langs` crates. Other crates are considered internal and may go through breaking changes in any release as long as the public API is unaffected. Try to keep the version in `tmc-langs-cli`'s `Cargo.toml` up to date so that CLI's help message contains the right version.

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.