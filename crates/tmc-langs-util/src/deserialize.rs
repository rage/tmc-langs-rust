use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::Read;

pub type JsonError = serde_path_to_error::Error<serde_json::Error>;

pub fn json_from_str<T: DeserializeOwned>(json: &str) -> Result<T, JsonError> {
    let de = &mut serde_json::Deserializer::from_str(json);
    let res = serde_path_to_error::deserialize(de)?;
    Ok(res)
}

pub fn json_from_reader<T: DeserializeOwned>(json: impl Read) -> Result<T, JsonError> {
    let de = &mut serde_json::Deserializer::from_reader(json);
    let res = serde_path_to_error::deserialize(de)?;
    Ok(res)
}

pub fn json_from_slice<T: DeserializeOwned>(json: &[u8]) -> Result<T, JsonError> {
    let de = &mut serde_json::Deserializer::from_slice(json);
    let res = serde_path_to_error::deserialize(de)?;
    Ok(res)
}

pub fn json_from_value<T: DeserializeOwned>(json: Value) -> Result<T, JsonError> {
    let res = serde_path_to_error::deserialize(json)?;
    Ok(res)
}

pub type TomlError = serde_path_to_error::Error<toml::de::Error>;

pub fn toml_from_str<T: DeserializeOwned>(toml: &str) -> Result<T, TomlError> {
    let de = &mut toml::Deserializer::new(toml);
    let res = serde_path_to_error::deserialize(de)?;
    Ok(res)
}

pub type YamlError = serde_path_to_error::Error<serde_yaml::Error>;

pub fn yaml_from_str<T: DeserializeOwned>(yaml: &str) -> Result<T, YamlError> {
    let de = serde_yaml::Deserializer::from_str(yaml);
    let res = serde_path_to_error::deserialize(de)?;
    Ok(res)
}

pub fn yaml_from_reader<T: DeserializeOwned>(yaml: impl Read) -> Result<T, YamlError> {
    let de = serde_yaml::Deserializer::from_reader(yaml);
    let res = serde_path_to_error::deserialize(de)?;
    Ok(res)
}

pub fn yaml_from_slice<T: DeserializeOwned>(yaml: &[u8]) -> Result<T, YamlError> {
    let de = serde_yaml::Deserializer::from_slice(yaml);
    let res = serde_path_to_error::deserialize(de)?;
    Ok(res)
}
