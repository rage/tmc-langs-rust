CLI interface for tmc-langs. The documentation for the various commands and parameters are best seen from the CLI itself. For example, running `tmc-langs-cli help clean` will print out the information for the `clean` sub-command. Alternatively, `src/app.rs` contains the CLI definition.

## Running the CLI

Running the project with Cargo from the repository root:
```bash
cargo run -p tmc-langs-cli help
```

## API format

See the `Output` struct in `src/output.rs` for the type definition that is converted to JSON and printed for every command. See the `api` directory for some examples of what the result looks as JSON. The example files are used in unit tests so they should always be up to date and correct.

In general, the API is structured as follows: there is an `output-kind` field which can have one of several values. Each value corresponds to a kind of message that has some fields associated with it. For example, if the `output-kind` is `output-data`, the message will have the fields `status`, `message`, `result`, and `data`. The `data` field, the value of which is an object, will then contain the field `output-data-kind` which specifies the rest of the fields of the `data` object, and so on.

## Binary deployment and downloads

Binaries for the supported targets are built and the binaries deployed to Google Cloud when creating a GitHub release. The binaries are available at https://download.mooc.fi/tmc-langs-rust/, with each binary following the file name format `tmc-langs-cli-{target}-{version}(.exe)`, with the `.exe` suffix added for the Windows binaries. For a list of targets see the README at the repository root. For example, The 64-bit Linux binary for version 0.5.0 is available at https://download.mooc.fi/tmc-langs-rust/tmc-langs-cli-x86_64-unknown-linux-gnu-0.5.0.

## Environment variables

| var name                         | usage                                                          |
| -------------------------------- | -------------------------------------------------------------- |
| `TMC_LANGS_ROOT_URL`             | Sets the TMC server url.                                       |
| `TMC_LANGS_CONFIG_DIR`           | Sets the config directory.                                     |
| `TMC_LANGS_DEFAULT_PROJECTS_DIR` | Sets the default projects directory.                           |
| `TMC_SANDBOX`                    | If set, the CLI considers itself to be running in tmc-sandbox. |
