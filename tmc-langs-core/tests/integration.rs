use dotenv::dotenv;
use std::env;
use tmc_langs_core::TmcCore;

#[test]
#[ignore]
fn get_organizations() {
    dotenv().ok();
    let email = env::var("EMAIL").unwrap();
    let password = env::var("PASSWORD").unwrap();

    let mut core = TmcCore::new_in_config(
        "https://tmc.mooc.fi".to_string(),
        "vscode_plugin".to_string(),
        "test".to_string(),
    )
    .unwrap();
    core.authenticate("vscode_plugin", email, password).unwrap();
    let _cd = core.get_course_details(600).unwrap();
}
