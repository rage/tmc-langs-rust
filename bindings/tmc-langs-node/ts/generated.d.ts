export interface StyleValidationResult {
  strategy: StyleValidationStrategy;
  validationErrors: { [key: string]: StyleValidationError[] } | null;
}

export interface StyleValidationError {
  column: number;
  line: number;
  message: string;
  sourceName: string;
}

export type StyleValidationStrategy = "FAIL" | "WARN" | "DISABLED";

export interface ExercisePackagingConfiguration {
  student_file_paths: string[];
  exercise_file_paths: string[];
}

export interface LocalExercise {
  "exercise-slug": string;
  "exercise-path": string;
}

export type OutputFormat = "Tar" | "Zip" | "TarZstd";

export interface RefreshData {
  "new-cache-path": string;
  "course-options": object;
  exercises: RefreshExercise[];
}

export interface RefreshExercise {
  name: string;
  checksum: string;
  points: string[];
  "sandbox-image": string;
  "tmcproject-yml": TmcProjectYml | null;
}

export interface TmcProjectYml {
  extra_student_files: string[] | null;
  extra_exercise_files: string[] | null;
  force_update: string[] | null;
  tests_timeout_ms: number | null;
  fail_on_valgrind_error: boolean | null;
  minimum_python_version: PythonVer | null;
  sandbox_image: string | null;
}

export interface PythonVer {
  major: number | null;
  minor: number | null;
  patch: number | null;
}

export interface RunResult {
  status: RunStatus;
  testResults: TestResult[];
  logs: { [key: string]: string };
}

export type RunStatus =
  | "PASSED"
  | "TESTS_FAILED"
  | "COMPILE_FAILED"
  | "TESTRUN_INTERRUPTED"
  | "GENERIC_ERROR";

export interface TestResult {
  name: string;
  successful: boolean;
  points: string[];
  message: string;
  exception: string[];
}

export interface ExerciseDesc {
  name: string;
  tests: TestDesc[];
}

export interface TestDesc {
  name: string;
  points: string[];
}

export interface UpdatedExercise {
  id: number;
}

export interface DownloadOrUpdateCourseExercisesResult {
  downloaded: ExerciseDownload[];
  skipped: ExerciseDownload[];
  failed: [ExerciseDownload, string[]][] | null;
}

export interface ExerciseDownload {
  id: number;
  "course-slug": string;
  "exercise-slug": string;
  path: string;
}

export interface CombinedCourseData {
  details: CourseDetails;
  exercises: CourseExercise[];
  settings: CourseData;
}

export interface CourseDetails {
  id: number;
  name: string;
  title: string;
  description: string | null;
  details_url: string;
  unlock_url: string;
  reviews_url: string;
  comet_url: string;
  spyware_urls: string[];
  unlockables: string[];
  exercises: Exercise[];
}

export interface Exercise {
  id: number;
  name: string;
  locked: boolean;
  deadline_description: string | null;
  deadline: string | null;
  soft_deadline: string | null;
  soft_deadline_description: string | null;
  checksum: string;
  return_url: string;
  zip_url: string;
  returnable: boolean;
  requires_review: boolean;
  attempted: boolean;
  completed: boolean;
  reviewed: boolean;
  all_review_points_given: boolean;
  memory_limit: number | null;
  runtime_params: string[];
  valgrind_strategy: string;
  code_review_requests_enabled: boolean;
  run_tests_locally_action_enabled: boolean;
  latest_submission_url: string | null;
  latest_submission_id: number | null;
  solution_zip_url: string | null;
}

export interface CourseExercise {
  id: number;
  available_points: ExercisePoint[];
  awarded_points: string[];
  name: string;
  publish_time: string | null;
  solution_visible_after: string | null;
  deadline: string | null;
  soft_deadline: string | null;
  disabled: boolean;
  unlocked: boolean;
}

export interface ExercisePoint {
  id: number;
  exercise_id: number;
  name: string;
  requires_review: boolean;
}

export interface CourseData {
  name: string;
  hide_after: string | null;
  hidden: boolean;
  cache_version: number | null;
  spreadsheet_key: string | null;
  hidden_if_registered_after: string | null;
  refreshed_at: string | null;
  locked_exercise_points_visible: boolean;
  description: string | null;
  paste_visibility: number | null;
  formal_name: string | null;
  certificate_downloadable: boolean | null;
  certificate_unlock_spec: string | null;
  organization_id: number | null;
  disabled_status: string | null;
  title: string | null;
  material_url: string | null;
  course_template_id: number | null;
  hide_submission_results: boolean;
  external_scoreboard_url: string | null;
  organization_slug: string | null;
}

export interface ExerciseDetails {
  course_name: string;
  course_id: number;
  code_review_requests_enabled: boolean;
  run_tests_locally_action_enabled: boolean;
  exercise_name: string;
  exercise_id: number;
  unlocked_at: string | null;
  deadline: string | null;
  submissions: ExerciseSubmission[];
}

export interface ExerciseSubmission {
  exercise_name: string;
  id: number;
  user_id: number;
  course_id: number;
  created_at: string;
  all_tests_passed: boolean;
  points: string | null;
  submitted_zip_url: string;
  paste_url: string | null;
  processing_time: number | null;
  reviewed: boolean;
  requests_review: boolean;
}

export interface Submission {
  id: number;
  user_id: number;
  pretest_error: string | null;
  created_at: string;
  exercise_name: string;
  course_id: number;
  processed: boolean;
  all_tests_passed: boolean;
  points: string | null;
  processing_tried_at: string | null;
  processing_began_at: string | null;
  processing_completed_at: string | null;
  times_sent_to_sandbox: number;
  processing_attempts_started_at: string;
  params_json: string | null;
  requires_review: boolean;
  requests_review: boolean;
  reviewed: boolean;
  message_for_reviewer: string;
  newer_submission_reviewed: boolean;
  review_dismissed: boolean;
  paste_available: boolean;
  message_for_paste: string;
  paste_key: string | null;
}

export interface UpdateResult {
  created: Exercise[];
  updated: Exercise[];
}

export interface Organization {
  name: string;
  information: string;
  slug: string;
  logo_path: string;
  pinned: boolean;
}

export interface Review {
  submission_id: number;
  exercise_name: string;
  id: number;
  marked_as_read: boolean;
  reviewer_name: string;
  review_body: string;
  points: string[];
  points_not_awarded: string[];
  url: string;
  update_url: string;
  created_at: string;
  updated_at: string;
}

export interface NewSubmission {
  show_submission_url: string;
  paste_url: string;
  submission_url: string;
}

export interface SubmissionFeedbackResponse {
  api_version: number;
  status: SubmissionStatus;
}

export type SubmissionStatus =
  | "processing"
  | "fail"
  | "ok"
  | "error"
  | "hidden";

export interface SubmissionFinished {
  api_version: number;
  all_tests_passed: boolean | null;
  user_id: number;
  login: string;
  course: string;
  exercise_name: string;
  status: SubmissionStatus;
  points: string[];
  valgrind: string | null;
  submission_url: string;
  solution_url: string | null;
  submitted_at: string;
  processing_time: number | null;
  reviewed: boolean;
  requests_review: boolean;
  paste_url: string | null;
  message_for_paste: string | null;
  missing_review_points: string[];
  test_cases: TestCase[] | null;
  feedback_questions: SubmissionFeedbackQuestion[] | null;
  feedback_answer_url: string | null;
  error: string | null;
  validations: StyleValidationResult | null;
}

export interface TestCase {
  name: string;
  successful: boolean;
  message: string | null;
  exception: string[] | null;
  detailed_message: string | null;
}

export interface SubmissionFeedbackQuestion {
  id: number;
  question: string;
  kind: SubmissionFeedbackKind;
}

export type SubmissionFeedbackKind = "Text" | {
  lower: number;
  upper: number;
};

export interface TmcConfig {
  projects_dir: string;
}