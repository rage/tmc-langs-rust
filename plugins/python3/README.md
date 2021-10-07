## Oldest maintained Python version

The plugin creates a warning when the Python used is detected to be older than the oldest maintained Python release as per https://devguide.python.org/#branchstatus. The minimum version is hard-coded and needs to be maintained manually. Next EOL: 3.6 in 2021-12-23.

## Updating tmc-python-tester

The tests use [tmc-python-tester](https://github.com/testmycode/tmc-python-tester) which should be updated occasionally by replacing `./tests/data/tmc` with the latest `./tmc` directory from the main branch of the repository. When updating, update the `UPDATED` file to indicate when the update was done.

## Student file policy

All files inside `./src` are considered student files, except for files with a `pyc` extension and files inside a `__pycache__` directory. In addition, all files in the project root with a `.py` extension are considered student files. All files in directories other than `./test` and `./tmc` are considered student files, except for files with a `pyc` extension and files inside a `__pycache__` directory.

### Example

```bash
# Student files
./src/file
./src/subdirectory/file
./root.py
./dir/any_non_cache_file

# Not student files
./test/test_file
./src/file.pyc
./src/__pycache__/file
./file_in_root
./dir/cache_file.pyc
./dir/__pycache__/cache_file
```

## Environment variables

| var name                | usage                       |
| ----------------------- | --------------------------- |
| `TMC_LANGS_PYTHON_EXEC` | Sets the Python executable. |
