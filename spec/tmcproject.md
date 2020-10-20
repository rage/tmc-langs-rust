Exercises can be configured by adding an optional `.tmcproject.yml` file to the exercise root directory.

## Keys

All of the keys listed below are optional.

| Key name               | Value type                             | Description                                                                                                                                                             |
| ---------------------- | -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| extra_student_files    | List of strings                        | List of file paths relative to the exercise root directory. The files and directories will be considered to be student files which should not be modified by tmc-langs. |
| extra_exercise_files   | List of strings                        | List of file paths relative to the exercise root directory. The files and directories will be considered to be exercise files which can be modified by tmc-langs.       |
| force_update           | List of strings                        | List of file paths relative to the exercise root directory. The files and directories can be modified by tmc-langs even if they are student files.                      |
| tests_timeout_ms       | Integer                                | The value is used to limit the running time of tests.                                                                                                                   |
| no_tests               | Boolean OR List of integers or strings | If set to true or a list, the no-tests plugin is used for the exercise. If set to a list, the list will be used as the exercise's points.                               |
| fail_on_valgrind_error | Boolean                                | If set, the C plugin will attempt to run valgrind and fail the exercise if it discovers errors.                                                                         |

## Example file contents

```yml
extra_student_files:
  ["file_in_exercise_root.py", "./another_file_in_exercise_root.py"]
extra_exercise_files: ["dir/file_in_subdirectory.xml"]
force_update: false
tests_timeout_ms: 1000
no_tests: false
fail_on_valgrind_error: false
```
