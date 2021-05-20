import * as langs from "./generated";

export class Token {
  access_token: string;
  token_type: string;
  scope: string;
}

export function initLogging(): null;
export function setEnv(key: string, value: string);

export function checkstyle(
  exercisePath: string,
  locale: string
): langs.StyleValidationResult | null;
export function clean(exercisePath: string): void;
export function compressProject(exercisePath: string, outputPath: string): void;
export function extractProject(archivePath: string, outputPath: string): void;
export function fastAvailablePoints(exercisePath: string): Array<string>;
export function findExercises(exercisePath: string): Array<string>;
export function getExercisePackagingConfiguration(
  exercisePath: string
): langs.ExercisePackagingConfiguration;
export function listLocalCourseExercises(
  clientName: string,
  courseSlug: string
): Array<langs.LocalExercise>;
export function prepareSolutions(
  exercisePath: string,
  outputPath: string
): void;
export function prepareStubs(exercisePath: string, outputPath: string): void;
export function prepareSubmission(
  outputFormat: langs.OutputFormat,
  clonePath: string,
  outputPath: string,
  stubZipPath: string | null,
  submissionPath: string,
  tmcParam: Map<string, Array<string>>,
  topLevelDirName: string | null
): void;
export function refreshCourse(
  cachePath: string,
  cacheRoot: string,
  courseName: string,
  gitBranch: string,
  sourceUrl: string
): langs.RefreshData;
export function runTests(exercisePath: string): langs.RunResult;
export function scanExercise(exercisePath: string): langs.ExerciseDesc;
export function checkExerciseUpdates(
  clientName: string,
  clientVersion: string
): Array<langs.UpdatedExercise>;
export function downloadModelSolution(
  clientName: string,
  clientVersion: string,
  exerciseId: number,
  target: string
): void;
export function downloadOldSubmission(
  clientName: string,
  clientVersion: string,
  submissionId: number,
  saveOldState: boolean,
  exerciseId: number,
  outputPath: string
): void;
export function downloadOrUpdateCourseExercises(
  clientName: string,
  clientVersion: string,
  downloadTemplate: boolean,
  exerciseId: Array<number>
): langs.DownloadOrUpdateCourseExercisesResult;
export function getCourseData(
  clientName: string,
  clientVersion: string,
  courseId: number
): langs.CombinedCourseData;
export function getCourseDetails(
  clientName: string,
  clientVersion: string,
  courseId: number
): langs.CourseDetails;
export function getCourseExercises(
  clientName: string,
  clientVersion: string,
  courseId: number
): Array<langs.CourseExercise>;
export function getCourseSettings(
  clientName: string,
  clientVersion: string,
  courseId: number
): langs.CourseData;
export function getCourses(
  clientName: string,
  clientVersion: string,
  organization: string
): Array<langs.CourseData>;
export function getExerciseDetails(
  clientName: string,
  clientVersion: string,
  exerciseId: number
): langs.ExerciseDetails;
export function getExerciseSubmissions(
  clientName: string,
  clientVersion: string,
  exerciseId: number
): Array<langs.Submission>;
export function getExerciseUpdates(
  clientName: string,
  clientVersion: string,
  courseId: number,
  exercise: Map<number, string>
): langs.UpdateResult;
export function getOrganization(
  clientName: string,
  clientVersion: string,
  organization: string
): langs.Organization;
export function getOrganizations(
  clientName: string,
  clientVersion: string
): Array<langs.Organization>;
export function getUnreadReviews(
  clientName: string,
  clientVersion: string,
  courseId: number
): Array<langs.Review>;
export function loggedIn(clientName: string, clientVersion: string): boolean;
export function login(
  clientName: string,
  clientVersion: string,
  base64: boolean,
  email: string | null,
  setAccessToken: string | null
): void;
export function logout(clientName: string, clientVersion: string): void;
export function markReviewAsRead(
  clientName: string,
  clientVersion: string,
  courseId: number,
  reviewId: number
): void;
export function paste(
  clientName: string,
  clientVersion: string,
  exerciseId: number,
  locale: string | null,
  pasteMessage: string | null,
  submissionPath: string
): langs.NewSubmission;
export function requestCodeReview(
  clientName: string,
  clientVersion: string,
  exerciseId: number,
  locale: string,
  messageForReviewer: string | null,
  submissionPath: string
): langs.NewSubmission;
export function resetExercise(
  clientName: string,
  clientVersion: string,
  saveOldState: boolean,
  exerciseId: number,
  exercisePath: string
): void;
export function sendFeedback(
  clientName: string,
  clientVersion: string,
  submissionId: number,
  feedback: Array<[number, string]>
): langs.SubmissionFeedbackResponse;
export function submit(
  clientName: string,
  clientVersion: string,
  dontBlock: boolean,
  locale: string | null,
  submissionPath: string,
  exerciseId: number
): langs.NewSubmission | langs.SubmissionFinished;
export function updateExercises(
  clientName: string,
  clientVersion: string
): langs.DownloadOrUpdateCourseExercisesResult;
export function waitForSubmission(
  clientName: string,
  clientVersion: string,
  submissionId: number
): langs.SubmissionFinished;
export function getSetting(clientName: string, setting: string): unknown;
export function listSettings(clientName: string): Map<string, unknown>;
export function migrateSettings(
  clientName: string,
  exercisePath: string,
  courseSlug: string,
  exerciseId: number,
  exerciseSlug: string,
  exerciseChecksum: string
): void;
export function moveProjectsDir(clientName: string, dir: string): void;
export function resetSettings(clientName: string): void;
export function setSetting(
  clientName: string,
  key: string,
  json: unknown
): void;
export function unsetSetting(clientName: string, setting: string): void;
