use crate::CoreError;
use crate::Organization;

use reqwest::Url;
use std::fs::File;
use std::path::Path;

/// A struct for interacting with the TestMyCode service, including authentication
pub struct TmcCore {
    api_url: Url,
}

impl TmcCore {
    pub fn new(api_url: &'static str) -> Self {
        // guarantee a trailing slash, otherwise join will drop the last component
        let api_url = Url::parse(&format!("{}/", api_url)).unwrap();
        Self { api_url }
    }

    /// Returns a list of organizations.
    pub fn get_organizations(&self) -> Result<Vec<Organization>, CoreError> {
        // TODO: cache

        let url = self.api_url.join("org.json").unwrap();
        let res = reqwest::blocking::get(url)?.json()?;
        Ok(res)
    }

    pub fn authenticate(&self, username: String, password: String) {
        let url = self.api_url.join("oauth/token").unwrap();
        todo!()
    }

    pub fn download_exercise(
        &self,
        id: u32,
        organization_slug: String,
        target: &Path,
    ) -> Result<(), CoreError> {
        // download zip
        let archive_path = target.join(format!("{}.zip", id));
        let mut archive = File::create(archive_path)?;
        let url = self
            .api_url
            .join(&format!("core/exercises/{}/download", id))
            .unwrap();
        let mut res = reqwest::blocking::get(url)?;
        res.copy_to(&mut archive)?;

        // extract
        todo!()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // TODO: use mock server
    //#[test]
    fn gets_organizations() {
        let core = TmcCore::new("https://tmc.mooc.fi/api/v8");
        let orgs = core.get_organizations().unwrap();
        panic!("{:#?}", orgs);
    }
}
