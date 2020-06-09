mod api;

use crate::error::{CoreError, Result};
use crate::response::*;

use oauth2::basic::BasicClient;
use oauth2::prelude::*;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, ResourceOwnerPassword, ResourceOwnerUsername, TokenResponse,
    TokenUrl,
};
use reqwest::{blocking::Client, Url};
use serde::de::DeserializeOwned;
use std::fmt::Debug;
use std::path::PathBuf;
use url1::Url as Url1;

pub type Token =
    oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>;

/// A struct for interacting with the TestMyCode service, including authentication
pub struct TmcCore {
    client: Client,
    config_dir: PathBuf,
    api_url: Url,
    auth_url: Url,
    token: Option<Token>,
}

// TODO: cache API results?
impl TmcCore {
    pub fn new(config_dir: PathBuf, root_url: &'static str) -> Result<Self> {
        // guarantee a trailing slash, otherwise join will drop the last component
        let tmc_url = Url::parse(&format!("{}/", root_url))?;
        let api_url = tmc_url.join("api/v8/")?;
        let auth_url = tmc_url.join("oauth/token")?;
        Ok(Self {
            client: Client::new(),
            config_dir,
            api_url,
            auth_url,
            token: None,
        })
    }

    pub fn new_in_config(root_url: &'static str) -> Result<Self> {
        let config_dir = dirs::cache_dir().ok_or(CoreError::HomeDir)?;
        Self::new(config_dir, root_url)
    }

    /// Attempts to log in with the given credentials, returns an error if an authentication token is already present.
    /// Username can be the user's username or email.
    pub fn authenticate(
        &mut self,
        client_name: &str,
        username: String,
        password: String,
    ) -> Result<()> {
        if self.token.is_some() {
            return Err(CoreError::AlreadyAuthenticated);
        }

        let url = self
            .api_url
            .join(&format!("application/{}/credentials", client_name))
            .unwrap();
        let credentials: Credentials = self.request_json(url)?;

        let auth_url = Url1::parse(self.auth_url.as_str()).unwrap();
        log::debug!("authenticating at {}", auth_url);
        let client = BasicClient::new(
            ClientId::new(credentials.application_id),
            Some(ClientSecret::new(credentials.secret)),
            AuthUrl::new(auth_url.clone()), // not used in the Resource Owner Password Credentials Grant
            Some(TokenUrl::new(auth_url)),
        );

        let token = client
            .exchange_password(
                &ResourceOwnerUsername::new(username),
                &ResourceOwnerPassword::new(password),
            )
            .map_err(|e| CoreError::Token(e))?;
        self.token = Some(token);
        log::debug!("authenticated");
        Ok(())
    }

    pub fn get_organizations(&self) -> Result<Vec<Organization>> {
        self.organizations()
    }

    pub fn send_diagnostics(&self) {
        todo!("https://tmc-bandicoot.testmycode.io?")
    }

    pub fn download_or_update_exercises(&self) {
        todo!()
    }

    pub fn get_course_details(&self) {
        todo!()
    }

    pub fn list_courses(&self) {
        todo!()
    }

    pub fn paste_with_comment(&self) {
        todo!()
    }

    pub fn run_checkstyle(&self) {
        todo!()
    }

    pub fn run_tests(&self) {
        todo!()
    }

    pub fn send_feedback(&self) {
        todo!()
    }

    pub fn send_snapshot_events(&self) {
        todo!("post to spyware urls")
    }

    pub fn submit(&self) {
        todo!()
    }

    pub fn get_exercise_updates(&self) {
        todo!()
    }

    pub fn mark_review_as_read(&self) {
        todo!()
    }

    pub fn get_unread_reviews(&self) {
        todo!()
    }

    pub fn request_code_review(&self) {
        todo!()
    }

    pub fn download_model_solution(&self) {
        todo!()
    }

    pub fn check_submission(&self, submission_url: &str) -> Result<SubmissionProcessingStatus> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = Url::parse(submission_url)?;
        let res: Response<SubmissionProcessingStatus> = self.request_json(url)?;
        let res = res.into_result()?;
        Ok(res)
    }

    fn request_json<T: DeserializeOwned + Debug>(&self, url: Url) -> Result<T> {
        log::debug!("requesting {}", url);
        let mut req = self.client.get(url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token.access_token().secret());
        }
        let res: Response<T> = req.send()?.json()?;
        log::trace!("received {:?}", res);
        res.into_result()
    }
}

// TODO: use mock server
#[cfg(test)]
mod test {
    use super::*;
    use serde_json::Value;

    const ROOT_URL: &'static str = "https://tmc.mooc.fi";

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn authenticated_core() -> TmcCore {
        dotenv::dotenv().ok();
        let user = std::env::var("TMC_USER").unwrap();
        let pass = std::env::var("TMC_PASS").unwrap();
        let mut core = TmcCore::new_in_config(ROOT_URL).unwrap();
        core.authenticate("vscode_plugin", user, pass).unwrap();
        core
    }

    #[test]
    #[ignore]
    fn authenticates() {
        init();

        dotenv::dotenv().ok();
        let user = std::env::var("TMC_USER").unwrap();
        let pass = std::env::var("TMC_PASS").unwrap();

        let mut core = TmcCore::new_in_config(ROOT_URL).unwrap();
        assert!(core.token.is_none());
        core.authenticate("vscode_plugin", user, pass).unwrap();
        assert!(core.token.is_some());
    }
}
