use crate::response::{CourseDetailsWrapper, Credentials, Response, Submission};
use crate::*;

use tmc_langs_util::task_executor;

use oauth2::basic::BasicClient;
use oauth2::prelude::*;
use oauth2::{
    AuthUrl, ClientId, ClientSecret, ResourceOwnerPassword, ResourceOwnerUsername, TokenResponse,
    TokenUrl,
};
use reqwest::{blocking::Client, Url};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fs::File;
use std::path::Path;
use url1::Url as Url1;

pub type Token =
    oauth2::StandardTokenResponse<oauth2::EmptyExtraTokenFields, oauth2::basic::BasicTokenType>;

/// A struct for interacting with the TestMyCode service, including authentication
pub struct TmcCore {
    client: Client,
    api_url: Url,
    auth_url: Url,
    token: Option<Token>,
}

// TODO: cache API results?
impl TmcCore {
    pub fn new(root_url: &'static str) -> Result<Self, CoreError> {
        // guarantee a trailing slash, otherwise join will drop the last component
        let tmc_url = Url::parse(&format!("{}/", root_url))?;
        let api_url = tmc_url.join("api/v8/")?;
        let auth_url = tmc_url.join("oauth/token")?;
        Ok(Self {
            client: Client::new(),
            api_url,
            auth_url,
            token: None,
        })
    }

    /// Attempts to log in with the given credentials, returns an error if an authentication token is already present.
    /// Username can be the user's username or email.
    pub fn authenticate(
        &mut self,
        client_name: &str,
        username: String,
        password: String,
    ) -> Result<(), CoreError> {
        if self.token.is_some() {
            return Err(CoreError::AlreadyAuthenticated);
        }

        let url = self
            .api_url
            .join(&format!("application/{}/credentials", client_name))
            .unwrap();
        let credentials: Credentials = self.request_json(url)?;

        let auth_url = Url1::parse(self.auth_url.as_str()).unwrap();
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

    pub fn unauthenticate(&mut self) {
        self.token = None
    }

    pub fn is_authenticated(&self) -> bool {
        self.token.is_some()
    }

    /// Returns a list of organizations.
    pub fn get_organizations(&self) -> Result<Vec<Organization>, CoreError> {
        let url = self.api_url.join("org.json")?;
        self.request_json(url)
    }

    /// Returns an organization.
    pub fn get_organization(&self, organization_slug: &str) -> Result<Organization, CoreError> {
        let url = self
            .api_url
            .join(&format!("org/{}.json", organization_slug))?;
        self.request_json(url)
    }

    pub fn get_courses(&self, organization_slug: &str) -> Result<Vec<Course>, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = self
            .api_url
            .join(&format!("core/org/{}/courses", organization_slug))
            .unwrap();
        self.request_json(url)
    }

    pub fn get_course_details(&self, course_id: usize) -> Result<CourseDetails, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = self
            .api_url
            .join(&format!("core/courses/{}", course_id))
            .unwrap();
        let wrapper: CourseDetailsWrapper = self.request_json(url)?;
        Ok(wrapper.course)
    }

    pub fn get_course_exercises(&self, course_id: usize) -> Result<Vec<CourseExercise>, CoreError> {
        if self.token.is_none() {
            return Err(CoreError::AuthRequired);
        }

        let url = self
            .api_url
            .join(&format!("courses/{}/exercises", course_id))
            .unwrap();
        self.request_json(url)
    }

    pub fn get_exercise_details(&self, course_id: usize) -> Result<ExerciseDetails, CoreError> {
        let url = self
            .api_url
            .join(&format!("core/exercises/{}", course_id))
            .unwrap();
        self.request_json(url)
    }

    pub fn download_exercise(
        &self,
        id: u32,
        organization_slug: &str,
        target: &Path,
    ) -> Result<(), CoreError> {
        // download zip
        let archive_path = target.join(format!("{}.zip", id));
        let mut archive = File::create(archive_path)?;
        let url = self
            .api_url
            .join(&format!("core/exercises/{}/download", id))
            .unwrap();

        log::debug!("requesting {}", url);
        let mut res = self.client.get(url).send()?;
        res.copy_to(&mut archive)?;

        // extract
        todo!()
    }

    pub fn submit_exercise(
        &self,
        exercise_id: usize,
        exercise_path: &Path,
    ) -> Result<(), CoreError> {
        // compress
        let compressed = task_executor::compress_project(exercise_path)?;

        let url = self
            .api_url
            .join(&format!("core/exercises/{}/submissions", exercise_id))
            .unwrap();

        let mut form = HashMap::new();
        form.insert("submission[file]", compressed);
        let res: Response<Submission> = self.client.post(url).form(&form).send()?.json()?;
        let submission = res.to_result()?;

        todo!();
        Ok(())
    }

    fn request_json<T: DeserializeOwned + Debug>(&self, url: Url) -> Result<T, CoreError> {
        log::debug!("requesting {}", url);
        let mut req = self.client.get(url);
        if let Some(token) = &self.token {
            req = req.bearer_auth(token.access_token().secret());
        }
        let res: Response<T> = req.send()?.json()?;
        log::trace!("received {:?}", res);
        res.to_result()
    }
}

// TODO: use mock server
#[cfg(test)]
mod test {
    use super::*;

    const ROOT_URL: &'static str = "https://tmc.mooc.fi";

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    fn authenticated_core() -> TmcCore {
        dotenv::dotenv().ok();
        let user = std::env::var("TMC_USER").unwrap();
        let pass = std::env::var("TMC_PASS").unwrap();
        let mut core = TmcCore::new(ROOT_URL).unwrap();
        core.authenticate("vscode_plugin", user, pass).unwrap();
        core
    }

    //#[test]
    fn authenticates() {
        init();

        dotenv::dotenv().ok();
        let user = std::env::var("TMC_USER").unwrap();
        let pass = std::env::var("TMC_PASS").unwrap();

        let mut core = TmcCore::new(ROOT_URL).unwrap();
        core.authenticate("vscode_plugin", user, pass).unwrap();
        assert!(core.token.is_some());
    }

    //#[test]
    fn gets_organizations() {
        init();

        let core = TmcCore::new(ROOT_URL).unwrap();
        let orgs = core.get_organization("mooc").unwrap();
        panic!("{:#?}", orgs);
    }

    // #[test]
    fn gets_courses() {
        init();

        let core = authenticated_core();
        let courses = core.get_courses("mooc").unwrap();
        panic!("{:#?}", courses);
    }

    //#[test]
    fn gets_course_details() {
        init();

        let core = authenticated_core();
        let course = core.get_course_details(588).unwrap();
        panic!("{:#?}", course);
    }

    //#[test]
    fn gets_course_exercises() {
        init();

        let core = authenticated_core();
        let course_exercises = core.get_course_exercises(588).unwrap();
        panic!("{:#?}", course_exercises);
    }

    //#[test]
    fn gets_exercise_details() {
        init();

        let core = authenticated_core();
        let exercise_details = core.get_exercise_details(81842).unwrap();
        panic!("{:#?}", exercise_details);
    }
}
