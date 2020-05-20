use super::{error::JavaPluginError, Error, TestCase, TestCaseStatus, TestRun};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tmc_langs_framework::domain::{RunResult, RunStatus, TestResult};

pub fn parse_java_home(properties: &str) -> Option<PathBuf> {
    for line in properties.lines() {
        if line.contains("java.home") {
            return line
                .split("=")
                .skip(1)
                .next()
                .map(|s| PathBuf::from(s.trim()));
        }
    }

    log::warn!("No java.home found in {}", properties);
    None
}

pub fn get_java_home() -> Result<PathBuf, Error> {
    let output = match Command::new("java")
        .arg("-XshowSettings:properties")
        .arg("-version")
        .output()
    {
        Ok(output) => output,
        Err(err) => return Err(Error::Plugin(Box::new(err))),
    };

    if !output.status.success() {
        return JavaPluginError::FailedCommand("java").into();
    }

    // information is printed to stderr
    let stderr = String::from_utf8_lossy(&output.stderr);

    match parse_java_home(&stderr) {
        Some(java_home) => Ok(java_home),
        None => JavaPluginError::NoJavaHome.into(),
    }
}

pub fn parse_test_result(results: &TestRun) -> Result<RunResult, Error> {
    let json = fs::read_to_string(&results.test_results)?;

    let mut test_results: Vec<TestResult> = vec![];
    let test_case_records: Vec<TestCase> = match serde_json::from_str(&json) {
        Ok(t) => t,
        Err(err) => return Err(Error::Plugin(Box::new(err))),
    };

    let mut status = RunStatus::Passed;
    for test_case in test_case_records {
        if test_case.status == TestCaseStatus::Failed {
            status = RunStatus::TestsFailed;
        }
        test_results.push(convert_test_case_result(test_case));
    }

    let mut logs = HashMap::new();
    logs.insert("stdout".to_string(), results.stdout.clone());
    logs.insert("stderr".to_string(), results.stderr.clone());
    Ok(RunResult {
        status,
        test_results,
        logs,
    })
}

fn convert_test_case_result(test_case: TestCase) -> TestResult {
    let mut exceptions = vec![];
    let mut points = vec![];

    test_case.exception.map(|e| {
        exceptions.push(e.message);
        for stack_trace in e.stack_trace {
            exceptions.push(stack_trace.to_string())
        }
    });

    points.extend(test_case.point_names);

    let name = format!("{} {}", test_case.class_name, test_case.method_name);
    let passed = test_case.status == TestCaseStatus::Passed;
    let message = test_case.message.unwrap_or_default();

    TestResult {
        name,
        passed,
        points,
        message,
        exceptions,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_java_home() {
        let properties = r#"Property settings:
    awt.toolkit = sun.awt.X11.XToolkit
    java.ext.dirs = /usr/lib/jvm/java-8-openjdk-amd64/jre/lib/ext
        /usr/java/packages/lib/ext
    java.home = /usr/lib/jvm/java-8-openjdk-amd64/jre
    user.timezone = 

openjdk version "1.8.0_252"
"#;

        let parsed = parse_java_home(properties);
        assert_eq!(
            Some(PathBuf::from("/usr/lib/jvm/java-8-openjdk-amd64/jre")),
            parsed,
        );
    }
}
