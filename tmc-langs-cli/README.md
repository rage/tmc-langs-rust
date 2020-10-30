CLI interface for tmc-langs. The documentation for the various commands and parameters are best seen from the CLI itself. For example, running `tmc-langs-cli help clean` will print out the information for the `clean` sub-command. Alternatively, `src/app.rs` contains the CLI definition.

## Environment variables

| var name               | usage                                                          |
| ---------------------- | -------------------------------------------------------------- |
| `TMC_LANGS_ROOT_URL`   | Sets the TMC server url.                                       |
| `TMC_LANGS_CONFIG_DIR` | Sets the config directory.                                     |
| `TMC_SANDBOX`          | If set, the CLI considers itself to be running in tmc-sandbox. |
