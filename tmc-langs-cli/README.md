CLI interface for tmc-langs. The documentation for the various commands and parameters are best seen from the CLI itself. For example, running `tmc-langs-cli help clean` will print out the information for the `clean` sub-command. Alternatively, `src/app.rs` contains the CLI definition.

## API format
See the `Output` struct in `src/output.rs` for the type definition that is converted to JSON and printed for every command. See the `api` directory for some examples of what the result looks as JSON. The example files are used in unit tests so they should always be up to date and correct.

In general, the API is structured as follows: there is an `output-kind` field which can have one of several values. Each value corresponds to a kind of message that has some fields associated with it. For example, if the `output-kind` is `output-data`, the message will have the fields `status`, `message`, `result`, and `data`. The `data` field, the value of which is an object, will then contain the field `output-data-kind` which specifies the rest of the fields of the `data` object, and so on.

## Environment variables

| var name               | usage                                                          |
| ---------------------- | -------------------------------------------------------------- |
| `TMC_LANGS_ROOT_URL`   | Sets the TMC server url.                                       |
| `TMC_LANGS_CONFIG_DIR` | Sets the config directory.                                     |
| `TMC_SANDBOX`          | If set, the CLI considers itself to be running in tmc-sandbox. |
