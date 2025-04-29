#![deny(clippy::print_stdout, clippy::print_stderr, clippy::unwrap_used)]

//! Used to communicate with the TMC server. See the TestMyCodeClient struct for more details.
//!
//! ```rust,no_run
//! use tmc_testmycode_client::TestMyCodeClient;
//!
//! let mut client = TestMyCodeClient::new("https://tmc.mooc.fi".parse().unwrap(), "some_client".to_string(), "some_version".to_string()).unwrap();
//! client.authenticate("client_name", "email".to_string(), "password".to_string());
//! let organizations = client.get_organizations();
//! ```
//!

mod client;
mod error;
pub mod request;
pub mod response;

pub use self::{
    client::{ClientUpdateData, TestMyCodeClient, Token, UpdateResult, api_v8},
    error::{TestMyCodeClientError, TestMyCodeClientResult},
};
// these types are part of tmc-testmycode-client's API and thus re-exported
pub use oauth2;
pub use tmc_langs_plugins::Language;
