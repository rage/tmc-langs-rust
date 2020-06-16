use mockito::{mock, Mock};
use std::env;
use std::io::Write;
use std::process::Stdio;
use std::process::{Command, Output};
use tmc_langs_core::Organization;

fn init() -> (Mock, Mock) {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "debug,hyper=warn,tokio_reactor=warn");
    }
    let _ = env_logger::builder().is_test(true).try_init();
    env::set_var("TMC_CORE_CLI_ROOT_URL", mockito::server_url());

    let m1 = mock("GET", "/api/v8/application/vscode_plugin/credentials")
        .with_body(
            serde_json::json!({
                "application_id": "id",
                "secret": "secret",
            })
            .to_string(),
        )
        .create();
    let m2 = mock("POST", "/oauth/token")
        .with_body(
            serde_json::json!({
                "access_token": "token",
                "token_type": "bearer",
            })
            .to_string(),
        )
        .create();
    (m1, m2)
}

fn run_cmd(args: &[&str]) -> Output {
    let path = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let path = path.parent().unwrap().join("tmc-langs-cli");
    let mut child = Command::new(path)
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .args(args)
        .spawn()
        .unwrap();
    let child_stdin = child.stdin.as_mut().unwrap();
    child_stdin.write_all("password\n".as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();

    log::debug!("stdout: {}", String::from_utf8_lossy(&out.stdout));
    log::debug!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    out
}

#[test]
fn get_organizations() {
    let _m = init();
    let _m = mock("GET", "/api/v8/org.json")
        .with_body(
            serde_json::json!([
                {
                    "name": "org name",
                    "information": "info",
                    "slug": "slg",
                    "logo_path": "path",
                    "pinned": false,
                }
            ])
            .to_string(),
        )
        .create();
    let out = run_cmd(&["core", "--email", "email", "get-organizations"]);
    let out = String::from_utf8(out.stdout).unwrap();
    let orgs: Vec<Organization> = serde_json::from_str(&out).unwrap();
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0].name, "org name");
}
