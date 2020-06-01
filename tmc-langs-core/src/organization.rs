use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Organization {
    name: String,
    information: String,
    slug: String,
    logo_path: String,
    pinned: bool,
}
