use dotenv::dotenv;
use std::env;
use tmc_client::TmcClient;

#[test]
#[ignore]
fn get_organizations() {
    dotenv().ok();
    let email = env::var("EMAIL").unwrap();
    let password = env::var("PASSWORD").unwrap();

    let mut client = TmcClient::new_in_config(
        "https://tmc.mooc.fi".to_string(),
        "vscode_plugin".to_string(),
        "test".to_string(),
    )
    .unwrap();
    client
        .authenticate("vscode_plugin", email, password)
        .unwrap();
    let _cd = client.get_course_details(600).unwrap();
}
