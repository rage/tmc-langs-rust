import os from "os";
import path from "path";
import fs from "fs";
import process from "process";
import functions from "../ts/functions";

var initiated = false;
function init() {
  if (!initiated) {
    initiated = true;
    functions.initLogging();
  }
  return process.env.TMC_LANGS_CONFIG_DIR;
}

var configDir: string | null = null;
async function setConfigDir(): Promise<string> {
  if (configDir == null) {
    configDir = await tempdir();
    functions.setEnv("TMC_LANGS_CONFIG_DIR", configDir);
  }
  return configDir;
}

async function writeFiles(root: string, files: ([string] | [string, string])[]) {
  for await (const [relative, contents] of files) {
    const p = path.join(root, relative);
    const dir = path.join(p, "../");
    await fs.promises.mkdir(dir, { recursive: true });
    if (contents == undefined) {
      await fs.promises.writeFile(p, "");
    } else {
      await fs.promises.writeFile(p, contents);
    }
  }
}

async function tempdir(): Promise<string> {
  return fs.promises.mkdtemp(path.join(os.tmpdir(), "langs_jest_"));
}

async function pythonDir() {
  const dir = await tempdir();
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
  init();

  const res = functions.checkstyle("jest/maven-exercise", "fin");
  expect(res?.strategy).toEqual("FAIL");
  expect(res?.validationErrors?.length).toBeFalsy();
});

test("cleans", async () => {
  init();

  const dir = await pythonDir();
  expect(fs.existsSync(path.join(dir, "__pycache__/cachefile"))).toBeTruthy();
  functions.clean(dir);
  expect(fs.existsSync(path.join(dir, "__pycache__/cachefile"))).toBeFalsy();
});

test("compresses project", async () => {
  init();

  const dir = await pythonDir();
  expect(fs.existsSync(path.join(dir, "output.zip"))).toBeFalsy();
  functions.compressProject(dir, path.join(dir, "output.zip"));
  expect(fs.existsSync(path.join(dir, "output.zip"))).toBeTruthy();
});

test("extracts project", async () => {
  init();

  const dir = await tempdir();
  expect(fs.existsSync(path.join(dir, "setup.py"))).toBeFalsy();
  functions.extractProject("jest/python-exercise.zip", dir);
  expect(fs.existsSync(path.join(dir, "setup.py"))).toBeTruthy();
});

test("finds points", async () => {
  init();

  const dir = await pythonDir();
  const points = functions.fastAvailablePoints(dir);
  expect(points[0]).toEqual("1");
});

test("finds exercises", async () => {
  init();

  const dir = await pythonDir();
  const points = functions.findExercises(dir);
  expect(points[0]).toEqual(dir);
});

test("gets exercise packaging configuration", async () => {
  init();

  const dir = await pythonDir();
  const config = functions.getExercisePackagingConfiguration(dir);
  expect(config.student_file_paths[0]).toEqual("src");
  expect(config.exercise_file_paths).toContain("test");
  expect(config.exercise_file_paths).toContain("tmc");
});

test("lists local course exercises", async () => {
  init();
  const configDir = await setConfigDir();

  await writeFiles(configDir, [
    ["tmc-list/config.toml", `projects_dir = "${configDir}/projects"`],
    [
      "projects/course/course_config.toml",
      `
course = 'course'
[exercises."exercise"]
id = 1
checksum = '2'
`,
    ],
    ["projects/course/exercise/setup.py"],
  ]);
  const exercises = functions.listLocalCourseExercises("list", "course");
  console.log(exercises);
  expect(exercises[0]["exercise-slug"]).toEqual("exercise");
});

test("prepares solutions", async () => {
  init();

  const dir = await pythonDir();
  const temp = await tempdir();
  expect(fs.existsSync(path.join(temp, "src"))).toBeFalsy();
  functions.prepareSolutions(dir, temp);
  expect(fs.existsSync(path.join(temp, "src"))).toBeTruthy();
});

test("prepares stubs", async () => {
  init();

  const dir = await pythonDir();
  const temp = await tempdir();
  expect(fs.existsSync(path.join(temp, "src"))).toBeFalsy();
  functions.prepareStubs(dir, temp);
  expect(fs.existsSync(path.join(temp, "src"))).toBeTruthy();
});

test.skip("prepares submission", async () => {
  init();

  // not used by vscode and has a "complicated" setup, untested for now
});

test.skip("refreshes course", async () => {
  init();

  // not used by vscode and has a "complicated" setup, untested for now
});

test("runs tests", async () => {
  init();

  const res = functions.runTests("jest/maven-exercise");
  expect(res.status).toEqual("TESTS_FAILED");
});

test("scans exercise", async () => {
  init();

  const res = functions.scanExercise("jest/maven-exercise");
  expect(res.name).toEqual("maven-exercise");
});

test("checks exercise updates", async () => {
  init();
});

test("downloads model solutions", async () => {
  init();
});

test("downloads old submission", async () => {
  init();
});

test("downloads or updates course exercises", async () => {
  init();
});

test("gets course data", async () => {
  init();
});

test("gets course details", async () => {
  init();
});

test("gets course exercises", async () => {
  init();
});

test("gets course settings", async () => {
  init();
});

test("gets courses", async () => {
  init();
});

test("gets exercise details", async () => {
  init();
});

test("gets exercise submissions", async () => {
  init();
});

test("gets exercise updates", async () => {
  init();
});

test("gets organizations", async () => {
  init();
});

test("gets unread reviews", async () => {
  init();
});

test("checks login status", async () => {
  init();
});

test("logs in", async () => {
  init();
});

test("logs out", async () => {
  init();
});

test("marks review as read", async () => {
  init();
});

test("pastes", async () => {
  init();
});

test("requests code review", async () => {
  init();
});

test("resets exercise", async () => {
  init();
});

test("sends feedback", async () => {
  init();
});

test("submits", async () => {
  init();
});

test("updates exercises", async () => {
  init();
});

test("waits for submission", async () => {
  init();
});

test("gets setting", async () => {
  init();
});

test("lists settings", async () => {
  init();
});

test("migrates settings", async () => {
  init();
});

test("moves projects dir", async () => {
  init();
});

test("resets settings", async () => {
  init();
});

test("sets setting", async () => {
  init();
});

test("unsets setting", async () => {
  init();
});
