## Student file policy

All files inside `./R` are considered student files.

### Example

```bash
# Student files
./R/file
./R/subdirectory/file

# Not student files
./root_file
./tests/test_file
```

### Updating tmc-r-tester

The [tmcRtestrunner](https://github.com/testmycode/tmc-r-tester/) library is included in `./tests` for easy installation during CI. The library should be checked for updates occasionally. When updating, update the `UPDATED` file to indicate when the update was done.
