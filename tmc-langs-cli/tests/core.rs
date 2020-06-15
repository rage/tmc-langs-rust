use httpmock::Method;
use httpmock::{mock, with_mock_server};
use std::env;
use std::process::{Command, Output};
use tmc_langs_core::Organization;

fn init() {
    let _ = env_logger::builder().is_test(true).try_init();
    env::set_var("TMC_CORE_ROOT_URL", "http://localhost:5000");
    mock(Method::GET, "/api/v8/application/vscode_plugin/credentials")
        .return_status(200)
        .return_json_body(&serde_json::json!({
            "application_id": "id",
            "secret": "s",
        }))
        .create();
    mock(Method::POST, "/oauth/token")
        .return_status(200)
        .return_json_body(&serde_json::json!({
            "access_token": "a",
            "token_type": "bearer",
        }))
        .create();
}

fn run_cmd(args: &[&str]) -> Output {
    let path = env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let path = path.parent().unwrap().join("tmc-langs-cli");
    let out = Command::new(path).args(args).output().unwrap();
    log::debug!("stdout: {}", String::from_utf8_lossy(&out.stdout));
    log::debug!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    out
}

//#[test]
#[with_mock_server]
fn core() {
    init();
    mock(Method::GET, "/api/v8/org.json")
        .return_status(200)
        .return_json_body(
            &serde_json::to_value(&serde_json::json!([
                {
                    "name": "n",
                    "information": "i",
                    "slug": "s",
                    "logo_path": "l",
                    "pinned": "p",
                }
            ]))
            .unwrap(),
        )
        .create();
    let out = run_cmd(&["core", "--email", "email", "get-organizations"]);
    let out = String::from_utf8(out.stdout).unwrap();
    let _orgs: Vec<Organization> = serde_json::from_str(&out).unwrap();
}
