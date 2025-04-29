use chrono::{DateTime, Utc};
use mooc_langs_api as api;
use serde::{Deserialize, Serialize};
use tmc_langs_util::{JsonError, deserialize};
#[cfg(feature = "ts-rs")]
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct TmcExerciseSlide {
    pub slide_id: Uuid,
    pub exercise_id: Uuid,
    pub exercise_name: String,
    pub exercise_order_number: i32,
    pub deadline: Option<DateTime<Utc>>,
    pub tasks: Vec<TmcExerciseTask>,
}

impl TryFrom<api::ExerciseSlide> for TmcExerciseSlide {
    type Error = JsonError;
    fn try_from(value: api::ExerciseSlide) -> Result<Self, Self::Error> {
        let slide = Self {
            slide_id: value.slide_id,
            exercise_id: value.exercise_id,
            exercise_name: value.exercise_name,
            exercise_order_number: value.exercise_order_number,
            deadline: value.deadline,
            tasks: value
                .tasks
                .into_iter()
                .map(TryFrom::try_from)
                .collect::<Result<_, _>>()?,
        };
        Ok(slide)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct TmcExerciseTask {
    pub task_id: Uuid,
    pub order_number: i32,
    pub assignment: serde_json::Value,
    pub public_spec: Option<PublicSpec>,
    pub model_solution_spec: Option<ModelSolutionSpec>,
}

impl TryFrom<api::ExerciseTask> for TmcExerciseTask {
    type Error = JsonError;
    fn try_from(value: api::ExerciseTask) -> Result<Self, Self::Error> {
        let task = Self {
            task_id: value.task_id,
            order_number: value.order_number,
            assignment: value.assignment,
            public_spec: value
                .public_spec
                .map(deserialize::json_from_value)
                .transpose()?,
            model_solution_spec: value
                .model_solution_spec
                .map(deserialize::json_from_value)
                .transpose()?,
        };
        Ok(task)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum PublicSpec {
    Browser {
        files: Vec<ExerciseFile>,
    },
    Editor {
        #[serde(rename = "archiveName")]
        archive_name: String,
        #[serde(rename = "archiveDownloadUrl")]
        archive_download_url: String,
        checksum: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
pub enum UserAnswer {
    Browser {
        files: Vec<ExerciseFile>,
    },
    Editor {
        #[serde(rename = "archiveDownloadUrl")]
        download_url: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub enum ModelSolutionSpec {
    Browser {
        #[serde(rename = "solutionFiles")]
        solution_files: Vec<ExerciseFile>,
    },
    Editor {
        #[serde(rename = "archiveDownloadUrl")]
        download_url: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts-rs", derive(TS))]
pub struct ExerciseFile {
    filepath: String,
    contents: String,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn deserializes_browser_public_spec() {
        let browser_task = r#"
{
    "type": "browser",
    "files": [
        {
            "filepath": "1",
            "contents": "2"
        },
        {
            "filepath": "3",
            "contents": "4"
        }
    ]
}
"#;
        serde_json::from_str::<PublicSpec>(browser_task).unwrap();
    }

    #[test]
    fn deserializes_editor_public_spec() {
        let editor_task = r#"
{
    "type": "editor",
    "archiveName": "1",
    "archiveDownloadUrl": "2",
    "checksum": "abcd"
}
"#;
        serde_json::from_str::<PublicSpec>(editor_task).unwrap();
    }
}
