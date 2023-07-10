#!/bin/bash

cargo test --features ts-rs generate_cli_bindings -- --ignored
echo "Generated bindings to ./crates/tmc-langs-cli/bindings.d.ts"