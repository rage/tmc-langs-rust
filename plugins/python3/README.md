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
