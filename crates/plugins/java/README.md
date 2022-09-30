## Java plugins

This crate differs from the others in that it contains two different plugins with some shared functionality: one for Maven, one for Ant. The common functionality is implemented in a JavaPlugin trait.

The `./deps` directory contains some Java dependencies, such as a bundled Maven for use by the Maven plugin.

- Maven: https://maven.apache.org/download.cgi
- j4rs: https://github.com/astonbitecode/j4rs/releases
- tmc-checkstyle-runner:
  - https://github.com/testmycode/tmc-checkstyle-runner/blob/master/pom.xml
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-checkstyle-runner/3.0.3-SNAPSHOT/maven-metadata.xml
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-checkstyle-runner/3.0.3-SNAPSHOT/tmc-checkstyle-runner-3.0.3-20200520.064542-3.jar
- tmc-junit-runner:
  - https://github.com/testmycode/tmc-junit-runner/blob/master/pom.xml
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-junit-runner/0.2.9-SNAPSHOT/maven-metadata.xml
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-junit-runner/0.2.9-SNAPSHOT/tmc-junit-runner-0.2.9-20200609.211712-1.jar

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
