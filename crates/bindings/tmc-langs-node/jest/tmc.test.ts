import os from "os";
import fs, { existsSync } from "fs";
import functions from "../ts/functions";
import { Tmc } from "../ts/tmc";
import { env } from "process";

var initiated = false;
function init(): Tmc {
  if (!initiated) {
    initiated = true;
    functions.initLogging();
  }

  var addr;
  if (env.TMC_LANGS_MOCK_SERVER_ADDR) {
    addr = env.TMC_LANGS_MOCK_SERVER_ADDR;
  } else {
    addr = `http://localhost:3000`;
  }
  console.log(`Running test against ${addr}`)
  const tmc = new Tmc("mock-client", "mock-version", addr);
  tmc.loginWithToken("");
  return tmc;
}

async function initWithTempDir(): Promise<Tmc> {
  if (!initiated) {
    initiated = true;
    functions.initLogging();
  }

  const tempDir = await tempdir();

  var addr;
  if (env.TMC_LANGS_MOCK_SERVER_ADDR) {
    addr = env.TMC_LANGS_MOCK_SERVER_ADDR;
  } else {
    addr = `http://localhost:3000`;
  }
  console.log(`Running test against ${addr}`)
  const tmc = new Tmc(
    "mock-client",
    "mock-version",
    addr,
    [tempDir, "config"].join("/"),
    [tempDir, "projects"].join("/")
  );
  tmc.loginWithToken("");
  return tmc;
}

async function writeFiles(
  root: string,
  files: ([string] | [string, string])[]
) {
  for await (const [relative, contents] of files) {
    const p = [root, relative].join("/");
    await fs.promises.mkdir(p, { recursive: true });
    await fs.promises.rmdir(p);
    if (contents == undefined) {
      await fs.promises.writeFile(p, "");
    } else {
      await fs.promises.writeFile(p, contents);
    }
  }
}

async function tempdir(): Promise<string> {
  return fs.promises.mkdtemp(
    [os.tmpdir().split("\\").join("/"), "langs_jest_"].join("/")
  );
}

async function mockEnvironment(tmc: Tmc) {
  const clientConfigDir = [tmc.configDir!, `tmc-${tmc.clientName}`].join("/");
  const clientProjectsDir = [tmc.defaultProjectsDir!, tmc.clientName].join("/");
  console.log("creating mock config at " + clientConfigDir);
  await writeFiles(clientConfigDir, [
    [
      "config.toml",
      `
projects_dir = "${clientProjectsDir}"
setting = "value"
`,
    ],
  ]);

  console.log("creating mock projects dir at " + clientProjectsDir);
  await writeFiles(clientProjectsDir, [
    [
      "some course/course_config.toml",
      `
course = 'some course'

[exercises."on disk exercise with update and submission"]
id = 1
checksum = 'old checksum'

[exercises."on disk exercise without update"]
id = 2
checksum = 'new checksum'
`,
    ],
  ]);
  await mockExercise(
    [
      clientProjectsDir,
      "some course/on disk exercise with update and submission",
    ].join("/")
  );
  await mockExercise(
    [clientProjectsDir, "some course/on disk exercise without update"].join("/")
  );
}

async function mockExercise(dir?: string): Promise<string> {
  if (!dir) {
    dir = await tempdir();
  }
  await writeFiles(dir, [
    ["setup.py"],
    ["src/main.py", "fn main:\
    pass"],
    ["test/test.py", "\
@Points('1')\
fn test:\
  pass"],
    ["__pycache__/cachefile"],
  ]);
  return dir;
}

test("checks style", async () => {
  const tmc = init();

  const res = tmc.checkstyle("jest/maven-exercise", "fin");
  expect(res?.strategy).toEqual("FAIL");
  expect(res?.validationErrors?.length).toBeFalsy();
});

test("cleans", async () => {
  const tmc = init();

  const dir = await mockExercise();
  expect(fs.existsSync([dir, "__pycache__/cachefile"].join("/"))).toBeTruthy();
  tmc.clean(dir);
  expect(fs.existsSync([dir, "__pycache__/cachefile"].join("/"))).toBeFalsy();
});

test("compresses project", async () => {
  const tmc = init();

  const dir = await mockExercise();
  expect(fs.existsSync([dir, "output.zip"].join("/"))).toBeFalsy();
  tmc.compressProject(dir, [dir, "output.zip"].join("/"), "zip", false, false);
  expect(fs.existsSync([dir, "output.zip"].join("/"))).toBeTruthy();
});

test("extracts project", async () => {
  const tmc = init();

  const dir = await tempdir();
  expect(fs.existsSync([dir, "setup.py"].join("/"))).toBeFalsy();
  tmc.extractProject("jest/python-exercise.zip", dir, "zip");
  expect(fs.existsSync([dir, "setup.py"].join("/"))).toBeTruthy();
});

test("finds points", async () => {
  const tmc = init();

  const dir = await mockExercise();
  const points = tmc.fastAvailablePoints(dir);
  expect(points[0]).toEqual("1");
});

test("finds exercises", async () => {
  const tmc = init();

  const rootDir = await tempdir();
  const dir = await mockExercise([rootDir, "some", "dirs"].join("/"));
  const exercises = tmc.findExercises(rootDir);
  expect(exercises[0]).toContain("some");
  expect(exercises[0]).toContain("dirs");
});

test("gets exercise packaging configuration", async () => {
  const tmc = init();

  const dir = await mockExercise();
  const config = tmc.getExercisePackagingConfiguration(dir);
  expect(config.student_file_paths).toContain("src/main.py");
  expect(config.exercise_file_paths).toContain("test/test.py");
});

test("lists local course exercises", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const exercises = tmc.listLocalCourseExercises("some course");
  expect(exercises.length).toBeGreaterThan(0);
});

test("prepares solutions", async () => {
  const tmc = init();

  const dir = await mockExercise();
  const temp = await tempdir();
  expect(fs.existsSync([temp, "src"].join("/"))).toBeFalsy();
  tmc.prepareSolution(dir, temp);
  expect(fs.existsSync([temp, "src"].join("/"))).toBeTruthy();
});

test("prepares stubs", async () => {
  const tmc = init();

  const dir = await mockExercise();
  const temp = await tempdir();
  expect(fs.existsSync([temp, "src"].join("/"))).toBeFalsy();
  tmc.prepareStub(dir, temp);
  expect(fs.existsSync([temp, "src"].join("/"))).toBeTruthy();
});

test.skip("prepares submission", async () => {
  const tmc = init();

  // not used by vscode and has a "complicated" setup, untested for now
});

test.skip("refreshes course", async () => {
  const tmc = init();

  // not used by vscode and has a "complicated" setup, untested for now
});

test("runs tests", async () => {
  const tmc = init();

  const testResult = tmc.runTests("jest/maven-exercise");
  expect(testResult.status).toEqual("TESTS_FAILED");
});

test("scans exercise", async () => {
  const tmc = init();

  const exercise = tmc.scanExercise("jest/maven-exercise");
  expect(exercise.name).toEqual("maven-exercise");
});

test("checks exercise updates", async () => {
  const tmc = await initWithTempDir();
  await mockEnvironment(tmc);

  const exercisesToUpdate = tmc.checkExerciseUpdates();
  expect(exercisesToUpdate.length).toBeGreaterThan(0);
});

test("downloads model solutions", async () => {
  const tmc = init();

  const temp = await tempdir();
  expect(existsSync([temp, "src"].join("/"))).toBeFalsy();
  tmc.downloadModelSolution(1, temp);
  expect(existsSync([temp, "src"].join("/"))).toBeTruthy();
});

test("downloads old submission", async () => {
  const tmc = init();

  const temp = await tempdir();
  expect(existsSync([temp, "src"].join("/"))).toBeFalsy();
  tmc.downloadOldSubmission(1, false, 1, temp);
  expect(existsSync([temp, "src"].join("/"))).toBeTruthy();
});

test("downloads or updates course exercises", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const res = tmc.downloadOrUpdateCourseExercises(false, [1, 2, 3, 4]);
  expect(res.downloaded.length).toBeGreaterThan(0);
});

test("gets course data", async () => {
  const tmc = init();

  const courseData = tmc.getCourseData(1);
  expect(courseData.exercises.length).toBeGreaterThan(0);
});

test("gets course details", async () => {
  const tmc = init();

  const courseDetails = tmc.getCourseDetails(1);
});

test("gets course exercises", async () => {
  const tmc = init();

  const courseExercises = tmc.getCourseExercises(1);
  expect(courseExercises.length).toBeGreaterThan(0);
});

test("gets course settings", async () => {
  const tmc = init();

  const courseSettings = tmc.getCourseSettings(1);
});

test("gets courses", async () => {
  const tmc = init();

  const courses = tmc.getCourses("org");
  expect(courses.length).toBeGreaterThan(0);
});

test("gets exercise details", async () => {
  const tmc = init();

  const exerciseDetails = tmc.getExerciseDetails(1);
});

test("gets exercise submissions", async () => {
  const tmc = init();

  const exerciseSubmissions = tmc.getExerciseSubmissions(1);
  expect(exerciseSubmissions.length).toBeGreaterThan(0);
});

test("gets exercise updates", async () => {
  const tmc = init();

  const map: [number, string][] = [
    [1, "old checksum"],
    [9999, "old checksum"],
  ];
  const exerciseUpdates = tmc.getExerciseUpdates(1, map);
  expect(exerciseUpdates.created.length).toBeGreaterThan(0);
  expect(exerciseUpdates.updated.length).toBeGreaterThan(0);
});

test("gets organizations", async () => {
  const tmc = init();

  const organizations = tmc.getOrganizations();
  expect(organizations.length).toBeGreaterThan(0);
});

test("gets unread reviews", async () => {
  const tmc = init();

  const unreadReviews = tmc.getUnreadReviews(1);
  expect(unreadReviews.length).toBeGreaterThan(0);
});

test("checks login status", async () => {
  const tmc = init();

  const loggedIn = tmc.loggedIn();
  expect(loggedIn).toBeTruthy();
});

test.skip("logs in", async () => {
  // difficult to mock oauth2 flow, untested
});

test("logs out", async () => {
  const tmc = init();

  expect(tmc.loggedIn()).toBeTruthy();
  tmc.logout();
  expect(tmc.loggedIn()).toBeFalsy();
});

test("marks review as read", async () => {
  const tmc = init();

  tmc.markReviewAsRead(1, 1);
});

test("pastes", async () => {
  const tmc = init();

  const dir = await mockExercise();
  const newSubmission = tmc.paste(1, null, null, dir);
});

test("requests code review", async () => {
  const tmc = init();

  const dir = await mockExercise();
  const newSubmission = tmc.requestCodeReview(1, "fin", null, dir);
});

test("resets exercise", async () => {
  const tmc = init();

  const dir = await mockExercise();
  tmc.resetExercise(false, 1, dir);
});

test("sends feedback", async () => {
  const tmc = init();

  const feedbackResponse = tmc.sendFeedback(1, [[1, "ans"]]);
});

test("submits", async () => {
  const tmc = init();

  const dir = await mockExercise();
  const newSubmission = tmc.submit(true, null, dir, 1);
});

test("updates exercises", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const updateResult = tmc.updateExercises();
  expect(updateResult.downloaded.length).toBeGreaterThan(0);
});

test("waits for submission", async () => {
  const tmc = init();

  const submissionFinished = tmc.waitForSubmission(1);
});

test("gets setting", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const value = tmc.getSetting("setting");
  expect(value).toEqual("value");
});

test("lists settings", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const settings = tmc.listSettings();
  expect(settings.setting).toEqual("value");
});

test("migrates settings", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const dir = await mockExercise();
  tmc.migrateExercise(dir, "new course", 1, "new exercise", "checksum");
});

test("moves projects dir", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const dir = await tempdir();
  tmc.moveProjectsDir(dir);
});

test("resets settings", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  tmc.resetSettings();
});

test("sets setting", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const value = tmc.getSetting("setting");
  expect(value).not.toEqual("hello!");
  tmc.setSetting("setting", JSON.stringify("hello!"));
  const hello = JSON.parse(tmc.getSetting("setting")!.toString());
  expect(hello).toEqual("hello!");
});

test("unsets setting", async () => {
  const tmc = await initWithTempDir();

  await mockEnvironment(tmc);
  const value = tmc.getSetting("setting");
  expect(value).not.toBeNull();
  tmc.unsetSetting("setting");
  const hello = tmc.getSetting("setting");
  expect(hello).toBeNull();
});
