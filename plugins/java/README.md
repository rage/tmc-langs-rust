## Java plugins

This crate differs from the others in that it contains two different plugins with some shared functionality: one for Maven, one for Ant. The common functionality is implemented in a JavaPlugin trait.

The `./deps` directory contains some Java dependencies, such as a bundled Maven for use by the Maven plugin.

## Maven

### Student file policy

All files inside `./src/main` are considered student files.

#### Example

```bash
# Student files
./src/main/file
./src/main/subdirectory/file

# Not student files
./tests/test_file
./src/not_main/file
```

## Ant

### Student file policy

All files inside `./src` are considered student files.

#### Example

```bash
# Student files
./src/file
./src/subdirectory/file

# Not student files
./tests/test_file
```
