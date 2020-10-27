Exercises can be configured by adding an optional `.tmcproject.yml` file to the exercise root directory.

## Keys

All of the keys listed below are optional.

| Key name               | Value type                             | Description                                                                                                                                                                          |
| ---------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| extra_student_files    | List of strings                        | List of file paths relative to the exercise root directory. The files and directories will be considered to be student files which should not be modified by tmc-langs.              |
| extra_exercise_files   | List of strings                        | List of file paths relative to the exercise root directory. The files and directories will be considered to be exercise files which can be modified by tmc-langs.                    |
| force_update           | List of strings                        | List of file paths relative to the exercise root directory. The files and directories are overwritten by tmc-langs during the update process even if they are student files.         |
| tests_timeout_ms       | Integer                                | The value is used to limit the running time of tests.                                                                                                                                |
| no_tests               | Boolean OR List of integers or strings | If set to true or a list, the no-tests plugin is used for the exercise. If set to a list, the list will be used as the exercise's points.                                            |
| fail_on_valgrind_error | Boolean                                | If set, the C plugin will attempt to run valgrind and fail the exercise if it discovers errors.                                                                                      |
| minimum_python_version | Python version string                  | Must be "{major}.{minor}.{patch}", "{major}.{minor}" or "{major}". If set, the Python plugin will warn the user if the Python version being used is below the given minimum version. |

## Example file contents

```yml
extra_student_files:
  - "file_in_exercise_root.py"
  - "./another_file_in_exercise_root.py"
extra_exercise_files:
  - "dir/file_in_subdirectory.xml"
force_update:
  - "./tests/forced_to_update"
tests_timeout_ms: 1000
no_tests: false
fail_on_valgrind_error: false
minimum_python_version: "3.8"
```

## Default student and exercise files

Each plugin defines some paths to be student or exercise files by default. To see the default settings for each plugin (called the plugin's _student file policy_), see each plugin's `README.md`:

- [C#](../plugins/csharp/README.md#student-file-policy)
- [Maven](../plugins/java/README.md#student-file-policy)
- [Ant](../plugins/java/README.md#student-file-policy-1)
- [Make](../plugins/make/README.md#student-file-policy)
- [No tests plugin](../plugins/notests/README.md#student-file-policy)
- [Python 3](../plugins/python3/README.md#student-file-policy)
- [R](../plugins/r/README.md#student-file-policy)
