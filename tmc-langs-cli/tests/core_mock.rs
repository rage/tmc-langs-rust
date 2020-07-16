use mockito::mock;
use std::env;
use std::process::Stdio;
use std::process::{Command, Output};
use tmc_langs_core::Organization;

fn init() {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "debug,hyper=warn,tokio_reactor=warn");
    }
    let _ = env_logger::builder().is_test(true).try_init();
    env::set_var("TMC_LANGS_ROOT_URL", mockito::server_url());
    env::set_var("TMC_LANGS_CONFIG_DIR", "./");
}

fn run_cmd(args: &[&str]) -> Output {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("client")).unwrap();
    let f = std::fs::File::create(temp.path().join("client").join("credentials.json")).unwrap();
    serde_json::to_writer(
        f,
        &serde_json::json! {
            {"access_token":"accesstoken","token_type":"bearer","scope":"public"}
        },
    )
    .unwrap();
    let path = env!("CARGO_BIN_EXE_tmc-langs-cli");
    let out = Command::new(path)
        .current_dir(temp.path())
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .args(args)
        .output()
        .unwrap();

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
    let out = run_cmd(&["core", "--client-name", "client", "get-organizations"]);
    assert!(out.status.success());
    let out = String::from_utf8(out.stdout).unwrap();
    let orgs: Vec<Organization> = serde_json::from_str(&out).unwrap();
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0].name, "org name");
}

//#[test]
fn _download_or_update_exercises() {
    let _m = init();
    let out = run_cmd(&[
        "core",
        "--email",
        "email",
        "download-or-update-exercises",
        "--exercise",
        "1234",
        "path1",
        "--exercise",
        "2345",
        "path2",
    ]);
    assert!(out.status.success());
    let _out = String::from_utf8(out.stdout).unwrap();
}
