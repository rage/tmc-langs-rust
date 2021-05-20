import * as tmc from "./functions";
import * as langs from "./generated";

export { langs };

export class Tmc {
  public clientName: string;
  public clientVersion: string;

  constructor(clientName: string, clientVersion: string) {
    this.clientName = clientName;
    this.clientVersion = clientVersion;
  }

  checkstyle(
    exercisePath: string,
    locale: string
  ): langs.StyleValidationResult | null {
    return tmc.checkstyle(exercisePath, locale);
  }

  clean(exercisePath: string): void {
    return tmc.clean(exercisePath);
  }

  compressProject(exercisePath: string, outputPath: string): void {
    return tmc.compressProject(exercisePath, outputPath);
  }

  extractProject(archivePath: string, outputPath: string): void {
    return tmc.extractProject(archivePath, outputPath);
  }

  fastAvailablePoints(exercisePath: string): Array<string> {
    return tmc.fastAvailablePoints(exercisePath);
  }

  findExercises(exercisePath: string): Array<string> {
    return tmc.findExercises(exercisePath);
  }

  getExercisePackagingConfiguration(
    exercisePath: string
  ): langs.ExercisePackagingConfiguration {
    return tmc.getExercisePackagingConfiguration(exercisePath);
  }

  listLocalCourseExercises(courseSlug: string): Array<langs.LocalExercise> {
    return tmc.listLocalCourseExercises(this.clientName, courseSlug);
  }

  prepareSolutions(exercisePath: string, outputPath: string): void {
    return tmc.prepareSolutions(exercisePath, outputPath);
  }

  prepareStubs(exercisePath: string, outputPath: string): void {
    return tmc.prepareStubs(exercisePath, outputPath);
  }

  prepareSubmission(
    outputFormat: langs.OutputFormat,
    clonePath: string,
    outputPath: string,
    stubZipPath: string | null,
    submissionPath: string,
    tmcParam: Map<string, Array<string>>,
    topLevelDirName: string | null
  ): void {
    return tmc.prepareSubmission(
      outputFormat,
      clonePath,
      outputPath,
      stubZipPath,
      submissionPath,
      tmcParam,
      topLevelDirName
    );
  }

  refreshCourse(
    cachePath: string,
    cacheRoot: string,
    courseName: string,
    gitBranch: string,
    sourceUrl: string
  ): langs.RefreshData {
    return tmc.refreshCourse(
      cachePath,
      cacheRoot,
      courseName,
      gitBranch,
      sourceUrl
    );
  }

  runTests(exercisePath: string): langs.RunResult {
    return tmc.runTests(exercisePath);
  }

  scanExercise(exercisePath: string): langs.ExerciseDesc {
    return tmc.scanExercise(exercisePath);
  }

  checkExerciseUpdates(): Array<langs.UpdatedExercise> {
    return tmc.checkExerciseUpdates(this.clientName, this.clientVersion);
  }

  downloadModelSolution(exerciseId: number, target: string): void {
    return tmc.downloadModelSolution(
      this.clientName,
      this.clientVersion,
      exerciseId,
      target
    );
  }

  downloadOldSubmission(
    submissionId: number,
    saveOldState: boolean,
    exerciseId: number,
    outputPath: string
  ): void {
    return tmc.downloadOldSubmission(
      this.clientName,
      this.clientVersion,
      submissionId,
      saveOldState,
      exerciseId,
      outputPath
    );
  }

  downloadOrUpdateCourseExercises(
    downloadTemplate: boolean,
    exerciseId: Array<number>
  ): langs.DownloadOrUpdateCourseExercisesResult {
    return tmc.downloadOrUpdateCourseExercises(
      this.clientName,
      this.clientVersion,
      downloadTemplate,
      exerciseId
    );
  }

  getCourseData(courseId: number): langs.CombinedCourseData {
    return tmc.getCourseData(this.clientName, this.clientVersion, courseId);
  }

  getCourseDetails(courseId: number): langs.CourseDetails {
    return tmc.getCourseDetails(this.clientName, this.clientVersion, courseId);
  }

  getCourseExercises(courseId: number): Array<langs.CourseExercise> {
    return tmc.getCourseExercises(
      this.clientName,
      this.clientVersion,
      courseId
    );
  }

  getCourseSettings(courseId: number): langs.CourseData {
    return tmc.getCourseSettings(this.clientName, this.clientVersion, courseId);
  }

  getCourses(organization: string): Array<langs.CourseData> {
    return tmc.getCourses(this.clientName, this.clientVersion, organization);
  }

  getExerciseDetails(exerciseId: number): langs.ExerciseDetails {
    return tmc.getExerciseDetails(
      this.clientName,
      this.clientVersion,
      exerciseId
    );
  }

  getExerciseSubmissions(exerciseId: number): Array<langs.Submission> {
    return tmc.getExerciseSubmissions(
      this.clientName,
      this.clientVersion,
      exerciseId
    );
  }

  getExerciseUpdates(
    courseId: number,
    exercise: Map<number, string>
  ): langs.UpdateResult {
    return tmc.getExerciseUpdates(
      this.clientName,
      this.clientVersion,
      courseId,
      exercise
    );
  }

  getOrganization(organization: string): langs.Organization {
    return tmc.getOrganization(
      this.clientName,
      this.clientVersion,
      organization
    );
  }

  getOrganizations(): Array<langs.Organization> {
    return tmc.getOrganizations(this.clientName, this.clientVersion);
  }

  getUnreadReviews(courseId: number): Array<langs.Review> {
    return tmc.getUnreadReviews(this.clientName, this.clientVersion, courseId);
  }

  loggedIn(): boolean {
    return tmc.loggedIn(this.clientName, this.clientVersion);
  }

  login(
    base64: boolean,
    email: string | null,
    setAccessToken: string | null
  ): void {
    return tmc.login(
      this.clientName,
      this.clientVersion,
      base64,
      email,
      setAccessToken
    );
  }

  logout(): void {
    return tmc.logout(this.clientName, this.clientVersion);
  }

  markReviewAsRead(courseId: number, reviewId: number): void {
    return tmc.markReviewAsRead(
      this.clientName,
      this.clientVersion,
      courseId,
      reviewId
    );
  }

  paste(
    exerciseId: number,
    locale: string | null,
    pasteMessage: string | null,
    submissionPath: string
  ): langs.NewSubmission {
    return tmc.paste(
      this.clientName,
      this.clientVersion,
      exerciseId,
      locale,
      pasteMessage,
      submissionPath
    );
  }

  requestCodeReview(
    exerciseId: number,
    locale: string,
    messageForReviewer: string | null,
    submissionPath: string
  ): langs.NewSubmission {
    return tmc.requestCodeReview(
      this.clientName,
      this.clientVersion,
      exerciseId,
      locale,
      messageForReviewer,
      submissionPath
    );
  }

  resetExercise(
    saveOldState: boolean,
    exerciseId: number,
    exercisePath: string
  ): void {
    return tmc.resetExercise(
      this.clientName,
      this.clientVersion,
      saveOldState,
      exerciseId,
      exercisePath
    );
  }

  sendFeedback(
    submissionId: number,
    feedback: Array<[number, string]>
  ): langs.SubmissionFeedbackResponse {
    return tmc.sendFeedback(
      this.clientName,
      this.clientVersion,
      submissionId,
      feedback
    );
  }

  submit(
    dontBlock: boolean,
    locale: string | null,
    submissionPath: string,
    exerciseId: number
  ): langs.NewSubmission | langs.SubmissionFinished {
    return tmc.submit(
      this.clientName,
      this.clientVersion,
      dontBlock,
      locale,
      submissionPath,
      exerciseId
    );
  }

  updateExercises(): langs.DownloadOrUpdateCourseExercisesResult {
    return tmc.updateExercises(this.clientName, this.clientVersion);
  }

  waitForSubmission(submissionId: number): langs.SubmissionFinished {
    return tmc.waitForSubmission(
      this.clientName,
      this.clientVersion,
      submissionId
    );
  }

  getSetting(setting: string): unknown {
    return tmc.getSetting(this.clientName, setting);
  }

  listSettings(): unknown {
    return tmc.listSettings(this.clientName);
  }

  migrateSettings(
    exercisePath: string,
    courseSlug: string,
    exerciseId: number,
    exerciseSlug: string,
    exerciseChecksum: string
  ): void {
    return tmc.migrateSettings(
      this.clientName,
      exercisePath,
      courseSlug,
      exerciseId,
      exerciseSlug,
      exerciseChecksum
    );
  }

  moveProjectsDir(dir: string): void {
    return tmc.moveProjectsDir(this.clientName, dir);
  }

  resetSettings(): void {
    return tmc.resetSettings(this.clientName);
  }

  setSetting(key: string, json: unknown): void {
    return tmc.setSetting(this.clientName, key, json);
  }

  unsetSetting(setting: string): void {
    return tmc.unsetSetting(this.clientName, setting);
  }
}
