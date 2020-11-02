## Oldest maintained Python version

The plugin creates a warning when the Python used is detected to be older than the oldest maintained Python release as per https://devguide.python.org/#branchstatus. The minimum version is hard-coded and needs to be maintained manually. Next EOL: 3.6 in 2021-12-23.

## Student file policy

All files inside `./src` are considered student files, except for files with a `pyc` extension and files inside a `__pycache__` directory. In addition, all files in the project root with a `.py` extension are considered student files.

### Example

```bash
# Student files
./src/file
./src/subdirectory/file
./root.py

# Not student files
./tests/test_file
./src/file.pyc
./src/__pycache__/file
```

## Environment variables

| var name                | usage                       |
| ----------------------- | --------------------------- |
| `TMC_LANGS_PYTHON_EXEC` | Sets the Python executable. |
