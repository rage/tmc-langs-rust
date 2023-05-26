## Building

Install Rust according to https://www.rust-lang.org/tools/install.

```bash
git clone git@github.com:rage/tmc-langs-rust.git
cd tmc-langs-rust
cargo build
```

If you have any troubles building the project, please do make an issue!

## Testing

Install [zstd 1.5.2](https://github.com/facebook/zstd). For example, on Ubuntu you need the package `libzstd1`. For Windows, download the appropriate archive from the [releases](https://github.com/facebook/zstd/releases), extract it and add the extracted directory to your PATH.

Install [Node.js and npm](https://docs.npmjs.com/downloading-and-installing-node-js-and-npm).

Install [Python 3](https://www.python.org/downloads/). You may wish to use [pyenv](https://github.com/pyenv/pyenv/) to manage different Python versions conveniently.

Install [OpenJDK 11](https://openjdk.java.net/install/index.html) (other versions may work as well) and [ant](https://ant.apache.org/).

Install [.NET 6.0](https://dotnet.microsoft.com/download).

Install [check](https://libcheck.github.io/check/) (works with at least 0.14 and 0.15), [valgrind](https://valgrind.org/) and `libsubunit0` (or equivalent for your distribution).

Install [R](https://www.r-project.org/), [devtools](https://devtools.r-lib.org/) by running either `sudo apt install r-cran-devtools` or `R -e 'install.packages("devtools", repos="http://cran.r-project.org"")'` and [tmc-r-tester](https://github.com/testmycode/tmc-rstudio) by running `R -e 'devtools::install_github("testmycode/tmc-r-tester/tmcRtestrunner", build = FALSE)'`. You can set the `R_LIBS_USER` environment variable to control where R packages get installed, for example by setting `export R_LIBS_USER="~/.R"` in your `.bashrc`. If you install `devtools` with the `R -e` command, it has several dependencies that need to be installed. For Ubuntu, they can be installed with

```bash
sudo apt install libcurl-dev libxml2-dev libopenssl-dev gcc-c++ libharfbuzz-dev libfribidi-dev libfreetype6-dev libpng-dev libtiff5-dev libjpeg-dev
```

With the dependencies instaled, the tests can be run with

```bash
cargo test
```

## Building and testing with Docker

The `docker.sh` script can be conveniently used to build and test the project. To build the binary and copy it out of the container to the project root, simply run `docker.sh`. To run tests, you can run `docker.sh "cargo test"`, or `docker.sh "cargo test -p tmc-langs-r"` and so on. The script also supports the special argument `interactive` to launch into an interactive bash shell inside the Docker container.

## Formatting and linting

Use `cargo +nightly fmt` and `cargo clippy` for formatting and linting. All crates should have the clippy lints `print_stdout` and `print_stderr` set to deny to allow the CLI to have total control over stdout and stderr. The CLI has one function where writing to stdout is allowed.

## Updating dependencies

For convenience, a tool called [cargo-outdated](https://crates.io/crates/cargo-outdated) can be installed to automatically check all the crates in the workspace for outdated dependencies. You may want to call `cargo update` first to update dependencies to the latest semver-compatible version.

In addition to the dependencies listed in each crate's `Cargo.toml`, the project bundles a few external dependencies such as `tmc-checkstyle-runner`, `tmc-junit-runner` and so on. When updating dependencies, you may want to check whether these projects have been updated.

## Versioning

tmc-langs-rust follows Rust-style semantic versioning, but only for the `tmc-langs-cli` and `tmc-langs` crates. Other crates are considered internal and may go through breaking changes in any release as long as the public API is unaffected. Try to keep the version in `tmc-langs-cli`'s `Cargo.toml` up to date so that CLI's help message contains the right version.

## Licensing

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
