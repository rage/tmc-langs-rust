## Java plugins

This crate differs from the others in that it contains two different plugins with some shared functionality: one for Maven, one for Ant. The common functionality is implemented in a JavaPlugin trait.

The `./deps` directory contains some Java dependencies, such as a bundled Maven for use by the Maven plugin.

- Maven: https://maven.apache.org/download.cgi
- j4rs: https://search.maven.org/artifact/io.github.astonbitecode/j4rs (Select version -> Downloads -> `jar-with-dependencies.jar`)
- tmc-checkstyle-runner:
  - https://github.com/testmycode/tmc-checkstyle-runner/blob/master/pom.xml (Find the `version`)
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-checkstyle-runner/{VERSION}/maven-metadata.xml (Find the latest `snapshotVersion.value`)
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-checkstyle-runner/{VERSION}/tmc-checkstyle-runner-{SNAPSHOT_VERSION}.jar
- tmc-junit-runner:
  - https://github.com/testmycode/tmc-junit-runner/blob/master/pom.xml (Find the `version`)
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-junit-runner/{VERSION}/maven-metadata.xml (Find the `snapshotVersion.value`)
  - https://maven.mooc.fi/snapshots/fi/helsinki/cs/tmc/tmc-junit-runner/{VERSION}/tmc-junit-runner-{SNAPSHOT_VERSION}.jar

Note that this plugin is not supported on musl due to dynamic loading not being supported on the platform.

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
