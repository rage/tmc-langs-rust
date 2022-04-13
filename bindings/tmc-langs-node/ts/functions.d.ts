import types, { Compression } from "./generated";

export class Token {
  access_token: string;
  token_type: string;
  scope: string;
}

export function initLogging(): null;
export function setEnv(key: string, value: string): null;

export function checkstyle(
  exercisePath: string,
  locale: string
): types.StyleValidationResult | null;
export function clean(exercisePath: string): void;
export function compressProject(exercisePath: string, outputPath: string, compression: Compression): void;
export function extractProject(archivePath: string, outputPath: string): void;
export function fastAvailablePoints(exercisePath: string): Array<string>;
export function findExercises(exercisePath: string): Array<string>;
export function getExercisePackagingConfiguration(
  exercisePath: string
): types.ExercisePackagingConfiguration;
export function listLocalCourseExercises(
  clientName: string,
  courseSlug: string
): Array<types.LocalExercise>;
export function prepareSolution(
  exercisePath: string,
  outputPath: string
): void;
export function prepareStub(exercisePath: string, outputPath: string): void;
export function prepareSubmission(
  outputFormat: types.Compression,
  clonePath: string,
  outputPath: string,
  stubZipPath: string | null,
  submissionPath: string,
  tmcParam: Array<[string, Array<string>]>,
  topLevelDirName: string | null
): void;
export function refreshCourse(
  cachePath: string,
  cacheRoot: string,
  courseName: string,
  gitBranch: string,
  sourceUrl: string
): types.RefreshData;
export function runTests(exercisePath: string): types.RunResult;
export function scanExercise(exercisePath: string): types.ExerciseDesc;
export function checkExerciseUpdates(
  clientName: string,
  clientVersion: string
): Array<types.UpdatedExercise>;
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
): types.DownloadOrUpdateCourseExercisesResult;
export function getCourseData(
  clientName: string,
  clientVersion: string,
  courseId: number
): types.CombinedCourseData;
export function getCourseDetails(
  clientName: string,
  clientVersion: string,
  courseId: number
): types.CourseDetails;
export function getCourseExercises(
  clientName: string,
  clientVersion: string,
  courseId: number
): Array<types.CourseExercise>;
export function getCourseSettings(
  clientName: string,
  clientVersion: string,
  courseId: number
): types.CourseData;
export function getCourses(
  clientName: string,
  clientVersion: string,
  organization: string
): Array<types.CourseData>;
export function getExerciseDetails(
  clientName: string,
  clientVersion: string,
  exerciseId: number
): types.ExerciseDetails;
export function getExerciseSubmissions(
  clientName: string,
  clientVersion: string,
  exerciseId: number
): Array<types.Submission>;
export function getExerciseUpdates(
  clientName: string,
  clientVersion: string,
  courseId: number,
  exercise: Array<[number, string]>
): types.UpdateResult;
export function getOrganization(
  clientName: string,
  clientVersion: string,
  organization: string
): types.Organization;
export function getOrganizations(
  clientName: string,
  clientVersion: string
): Array<types.Organization>;
export function getUnreadReviews(
  clientName: string,
  clientVersion: string,
  courseId: number
): Array<types.Review>;
export function loggedIn(clientName: string, clientVersion: string): boolean;
export function login(
  clientName: string,
  clientVersion: string,
  base64: boolean,
  email: string,
  password: string
): void;
export function loginWithToken(
  clientName: string,
  clientVersion: string,
  accessToken: string
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
): types.NewSubmission;
export function requestCodeReview(
  clientName: string,
  clientVersion: string,
  exerciseId: number,
  locale: string,
  messageForReviewer: string | null,
  submissionPath: string
): types.NewSubmission;
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
): types.SubmissionFeedbackResponse;
export function submit(
  clientName: string,
  clientVersion: string,
  dontBlock: boolean,
  locale: string | null,
  submissionPath: string,
  exerciseId: number
): types.NewSubmission | types.SubmissionFinished;
export function updateExercises(
  clientName: string,
  clientVersion: string
): types.DownloadOrUpdateCourseExercisesResult;
export function waitForSubmission(
  clientName: string,
  clientVersion: string,
  submissionId: number
): types.SubmissionFinished;
export function getSetting(clientName: string, setting: string): object | string | null;
export function listSettings(clientName: string): Record<string, object>;
export function migrateExercise(
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
