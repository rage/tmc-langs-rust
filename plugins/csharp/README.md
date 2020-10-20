## Student file policy

All files inside `./src` are considered student files, except for any files within `bin` or `obj` directories.

### Example

```bash
# Student files
./src/file
./src/subdirectory/file

# Not student files
./tests/test_file
./src/bin/binary
./src/any/obj/any/object_file
```

## Environment variables

| Variable name               | Description                                                                   |
| --------------------------- | ----------------------------------------------------------------------------- |
| `TMC_CSHARP_BOOTSTRAP_PATH` | Overrides the path to the TMC C# bootstrap's TestMyCode.CSharp.Bootstrap.dll. |
