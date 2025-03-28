# Course configuration options

The course can be configured as a whole using a configuration file called `course_options.yml`.

`course_options.yml` has to be in the root directory of the repository and may contain the following configurations:

- **`hidden`** hides the course from students (including in web UI and IDEs).
- **`hide_after`** hides the course after a given date from students (including in web UI and IDEs) and refuses to accept any more submissions.
- **`hidden_if_registered_after`** hides the course from students (including in web UI and IDEs) who have registered after the given date. After the given date it will also be invisible to unregistered users.
- **`locked_exercise_points_visible`** tells whether exercises that one has not unlocked are visible on the scoreboard. Defaults to true.
- **`formal_name`** course formal name to be used in certificates. Makes it easier to use the same template and repository for multiple courses.
- **`paste_visibility`** determines who can access pastes for submissions. Possible values:
  - `open` (default value):
    - In the first 2 hours after submission, the paste is visible to:
      - The submission owner.
      - Teachers and course assistants.
      - Users who have completed the exercise.
      - Anyone, if the submission did not pass all tests.
    - After 2 hours, the paste is only visible to:
      - The submission owner.
      - Users who have completed the exercise.
      - Teachers and course assistants.
  - `protected`: Visible only to:
    - The user who submitted it.
    - Teachers and course assistants.
  - `no-tests-public`:
    - In the first 2 hours after submission, the paste is visible to everyone.
    - After 2 hours, it is only visible to:
      - The submission owner.
      - Teachers and course assistants.
  - `everyone`: Always visible to everyone.
  - `completed-only`: Only visible to:
    - Users who have completed the exercise.
    - The submission owner.
    - Teachers and course assistants.
- **`certificate_downloadable`** makes the certificate downloadable. Defaults to false.
- **`certificate_unlock_spec`** makes the certificate downloadable only after the given condition is met. If `certificate_downloadable` is false then it overrides this. See below for more.

If there are multiple unlock conditions, then all of them must be met for the exercise to be unlocked.

The full syntax of a `certificate_unlock_spec` condition is as follows:

- `exercise [group] <exercise-or-group>`
- `point[s] <list-of-point-names>`
- `<N>% [in|of|from] <exercise-or-group>`
- `<N> point[s] <exercise-or-group>`
- `<N> exercise[s] [in|of|from] <exercise-or-group>`
- `<date-or-datetime>`

Here's an example configuration file:

```yaml
hidden: true
hide_after: 2010-02-15
hidden_if_registered_after: 2010-01-01
locked_exercise_points_visible: false
formal_name: "Object Oriented Programming – part 1"
paste_visibility: open
certificate_downloadable: true
certificate_unlock_spec: 80% of week1
```
