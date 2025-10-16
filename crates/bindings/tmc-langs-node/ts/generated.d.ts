export type StyleValidationResult = { strategy: StyleValidationStrategy, validation_errors: Record<string, Array<StyleValidationError>> | null, }

export type StyleValidationError = { column: number, line: number, message: string, source_name: string, }

export type StyleValidationStrategy = "FAIL" | "WARN" | "DISABLED";

export type ExercisePackagingConfiguration = { 
/**
 * Student folders or files which are copied from submission.
 */
student_file_paths: Array<string>,
/**
 * Exercise folders or files which are copied from exercise template or clone.
 */
exercise_file_paths: Array<string>, }

export type LocalTmcExercise = { "exercise-slug": string, "exercise-path": string, }

export type Compression = "tar" | "zip" | "zstd";

export type RefreshData = { "new-cache-path": string, "course-options": object, exercises: Array<RefreshExercise>, }

export type RefreshExercise = { name: string, checksum: string, points: Array<string>, "sandbox-image": string, "tmcproject-yml": TmcProjectYml | null, }

export type TmcProjectYml = { 
/**
 * A list of files or directories that will always be considered student files.
 */
extra_student_files: Array<string>,
/**
 * A list of files or directories that will always be considered exercise files.
 * `extra_student_files` takes precedence if a file is both an extra student file and an extra exercise file.
 */
extra_exercise_files: Array<string>,
/**
 * A list of files that should always be overwritten by updates even if they are student files.
 */
force_update: Array<string>,
/**
 * If set, tests are forcibly stopped after this duration.
 */
tests_timeout_ms?: number,
/**
 * If set, Valgrind errors will be considered test errors.
 */
fail_on_valgrind_error?: boolean,
/**
 * If set, will cause an error telling the student to update their Python if their version is older than the minimum.
 */
minimum_python_version?: PythonVer,
/**
 * Overrides the default sandbox image. e.g. `eu.gcr.io/moocfi-public/tmc-sandbox-python:latest`
 */
sandbox_image?: string,
/**
 * Overrides the default archive size limit (500 Mb).
 */
submission_size_limit_mb?: number, }

export type PythonVer = { major: number, minor: number | null, patch: number | null, }

export type RunResult = { 
/**
 * The overall status of a test run.
 */
status: RunStatus,
/**
 * Whether each test passed and which points were awarded.
 */
testResults: Array<TestResult>,
/**
 * Logs from the test run.
 * The key may be an arbitrary string identifying the type of log.
 */
logs: Record<string, string>, }

export type RunStatus = "PASSED" | "TESTS_FAILED" | "COMPILE_FAILED" | "TESTRUN_INTERRUPTED" | "GENERIC_ERROR";

export type TestResult = { name: string, successful: boolean,
/**
 * List of points that were received from the exercise from passed tests.
 */
points: Array<string>, message: string, exception: Array<string>, }

export type ExerciseDesc = { 
/**
 * The name of the exercise to be shown to the user.
 * Does not necessarily match or even contain the directory name.
 */
name: string,
/**
 * Descriptions of the tests that will be run for this exercise.
 */
tests: Array<TestDesc>, }

export type TestDesc = { 
/**
 * The full name of the test.
 *
 * If the language organises tests into suites or classes, it is customary
 * to name the test as "class_name.method_name".
 */
name: string,
/**
 * The list of point names that passing this test may give.
 *
 * To obtain a point X, the user must pass all exercises that require point X.
 */
points: Array<string>, }

export type UpdatedExercise = { id: number, }

export type DownloadOrUpdateTmcCourseExercisesResult = { downloaded: Array<TmcExerciseDownload>, skipped: Array<TmcExerciseDownload>, failed?: Array<[TmcExerciseDownload, Array<string>]>, }

export type TmcExerciseDownload = { id: number, "course-slug": string, "exercise-slug": string, path: string, }

export type CombinedCourseData = { details: CourseDetails, exercises: Array<CourseExercise>, settings: CourseData, }

export type CourseDetails = { unlockables: Array<string>, exercises: Array<Exercise>, id: number, name: string, title: string, description: string | null,
/**
 * /api/v8/core/courses/{course_id}
 */
details_url: string,
/**
 * /api/v8/core/courses/{course_id}/unlock
 */
unlock_url: string,
/**
 * /api/v8/core/courses/{course_id}/reviews
 */
reviews_url: string,
/**
 * Typically empty.
 */
comet_url: string, spyware_urls: Array<string>, }

export type Exercise = { id: number, name: string, locked: boolean, deadline_description: string | null, deadline: string | null, soft_deadline: string | null, soft_deadline_description: string | null, checksum: string,
/**
 * /api/v8/core/exercises/{exercise_id}/submissions
 */
return_url: string,
/**
 * /api/v8/core/exercises/{exercise_id}/download
 */
zip_url: string, returnable: boolean, requires_review: boolean, attempted: boolean, completed: boolean, reviewed: boolean, all_review_points_given: boolean, memory_limit: number | null, runtime_params: Array<string>, valgrind_strategy: string | null, code_review_requests_enabled: boolean, run_tests_locally_action_enabled: boolean,
/**
 * Typically null.
 */
latest_submission_url: string | null, latest_submission_id: number | null,
/**
 * /api/v8/core/exercises/{exercise_id}/solution/download
 */
solution_zip_url: string | null, }

export type CourseExercise = { id: number, available_points: Array<ExercisePoint>, awarded_points: Array<string>, name: string, publish_time: string | null, solution_visible_after: string | null, deadline: string | null, soft_deadline: string | null, disabled: boolean, unlocked: boolean, }

export type ExercisePoint = { id: number, exercise_id: number, name: string, requires_review: boolean, }

export type CourseData = { name: string, hide_after: string | null, hidden: boolean, cache_version: number | null, spreadsheet_key: string | null, hidden_if_registered_after: string | null, refreshed_at: string | null, locked_exercise_points_visible: boolean, description: string | null, paste_visibility: number | null, formal_name: string | null, certificate_downloadable: boolean | null, certificate_unlock_spec: string | null, organization_id: number | null, disabled_status: string | null, title: string | null,
/**
 * Typically empty.
 */
material_url: string | null, course_template_id: number | null, hide_submission_results: boolean,
/**
 * Typically empty.
 */
external_scoreboard_url: string | null, organization_slug: string | null, }

export type ExerciseDetails = { course_name: string, course_id: number, code_review_requests_enabled: boolean, run_tests_locally_action_enabled: boolean, exercise_name: string, exercise_id: number, unlocked_at: string | null, deadline: string | null, submissions: Array<ExerciseSubmission>, }

export type ExerciseSubmission = { exercise_name: string, id: number, user_id: number, course_id: number, created_at: string, all_tests_passed: boolean, points: string | null,
/**
 * /api/v8/core/submissions/{submission_id}/download
 */
submitted_zip_url: string,
/**
 * https://tmc.mooc.fi/paste/{paste_code}
 */
paste_url: string | null, processing_time: number | null, reviewed: boolean, requests_review: boolean, }

export type Submission = { id: number, user_id: number, pretest_error: string | null, created_at: string, exercise_name: string, course_id: number, processed: boolean, all_tests_passed: boolean, points: string | null, processing_tried_at: string | null, processing_began_at: string | null, processing_completed_at: string | null, times_sent_to_sandbox: number, processing_attempts_started_at: string, params_json: string | null, requires_review: boolean, requests_review: boolean, reviewed: boolean, message_for_reviewer: string, newer_submission_reviewed: boolean, review_dismissed: boolean, paste_available: boolean, message_for_paste: string, paste_key: string | null, }

export type UpdateResult = { created: Array<Exercise>, updated: Array<Exercise>, }

export type Organization = { name: string, information: string, slug: string, logo_path: string, pinned: boolean, }

export type Review = { submission_id: number, exercise_name: string, id: number, marked_as_read: boolean, reviewer_name: string, review_body: string, points: Array<string>, points_not_awarded: Array<string>,
/**
 * https://tmc.mooc.fi/submissions/{submission_id}/reviews
 */
url: string,
/**
 * /api/v8/core/courses/{course_id}/reviews/{review_id}
 */
update_url: string, created_at: string, updated_at: string, }

export type NewSubmission = { 
/**
 * https://tmc.mooc.fi/api/v8/core/submissions/{submission_id}
 */
show_submission_url: string,
/**
 * https://tmc.mooc.fi/paste/{paste_code}
 */
paste_url: string,
/**
 * https://tmc.mooc.fi/submissions/{submission_id}
 */
submission_url: string, }

export type SubmissionFeedbackResponse = { api_version: number, status: SubmissionStatus, }

export type SubmissionStatus = "processing" | "fail" | "ok" | "error" | "hidden";

export type TmcStyleValidationResult = { strategy: TmcStyleValidationStrategy, validationErrors: Record<string, Array<TmcStyleValidationError>> | null, }

export type TmcStyleValidationError = { column: number, line: number, message: string, sourceName: string, }

export type TmcStyleValidationStrategy = "FAIL" | "WARN" | "DISABLED";

export type SubmissionFinished = { api_version: number, all_tests_passed: boolean | null, user_id: number, login: string, course: string, exercise_name: string, status: SubmissionStatus, points: Array<string>, valgrind: string | null,
/**
 * https://tmc.mooc.fi/submissions/{submission_id}}
 */
submission_url: string,
/**
 * https://tmc.mooc.fi/exercises/{exercise_id}/solution
 */
solution_url: string | null, submitted_at: string, processing_time: number | null, reviewed: boolean, requests_review: boolean,
/**
 * https://tmc.mooc.fi/paste/{paste_code}
 */
paste_url: string | null, message_for_paste: string | null, missing_review_points: Array<string>, test_cases: Array<TestCase> | null, feedback_questions: Array<SubmissionFeedbackQuestion> | null,
/**
 * /api/v8/core/submissions/{submission_id}/feedback
 */
feedback_answer_url: string | null, error: string | null, validations: TmcStyleValidationResult | null, }

export type TestCase = { name: string, successful: boolean, message: string | null, exception: Array<string> | null, detailed_message: string | null, }

export type SubmissionFeedbackQuestion = { id: number, question: string, kind: SubmissionFeedbackKind, }

export type SubmissionFeedbackKind = "Text" | { "IntRange": { lower: number, upper: number, } };

export type TmcConfig = { projects_dir: string, }

export type CourseInstance = { id: string, course_id: string, course_slug: string, course_name: string, course_description: string | null, instance_name: string | null, instance_description: string | null, }

