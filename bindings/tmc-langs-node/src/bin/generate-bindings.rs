use std::fs::File;

fn main() {
    let mut file = File::create(concat!(env!("CARGO_MANIFEST_DIR"), "/ts/generated.d.ts")).unwrap();
    ts_rs::export_to! {
        &mut file,
        // checkstyle
        tmc_langs::StyleValidationResult,
        tmc_langs::StyleValidationError,
        tmc_langs::StyleValidationStrategy,
        // getExercisePackagingConfiguration
        tmc_langs::ExercisePackagingConfiguration,
        // listLocalCourseExercises
        tmc_langs::LocalExercise,
        // prepareSubmission
        tmc_langs::OutputFormat,
        // refreshCourse
        tmc_langs::RefreshData,
        tmc_langs::RefreshExercise,
        tmc_langs::TmcProjectYml,
        tmc_langs::PythonVer,
        // runTests
        tmc_langs::RunResult,
        tmc_langs::RunStatus,
        tmc_langs::TestResult,
        // scanExercise
        tmc_langs::ExerciseDesc,
        tmc_langs::TestDesc,
        // checkExerciseUpdates
        tmc_langs::UpdatedExercise,
        // downloadOrUpdateCourseExercises
        tmc_langs::DownloadOrUpdateCourseExercisesResult,
        tmc_langs::ExerciseDownload,
        // getCourseData
        tmc_langs::CombinedCourseData,
        // getCourseData
        tmc_langs::CourseDetails,
        tmc_langs::Exercise,
        // getCourseExercises
        tmc_langs::CourseExercise,
        tmc_langs::ExercisePoint,
        // getCourseSettings
        // getCourses
        tmc_langs::CourseData,
        // getExerciseDetails
        tmc_langs::ExerciseDetails,
        tmc_langs::ExerciseSubmission,
        // getExerciseSubmissions
        tmc_langs::Submission,
        // getExerciseUpdates
        tmc_langs::UpdateResult,
        // getOrganization
        // getOrganizations
        tmc_langs::Organization,
        // getUnreadReviews
        tmc_langs::Review,
        // paste
        // requestCodeReview
        tmc_langs::NewSubmission,
        // sendFeedback
        tmc_langs::SubmissionFeedbackResponse,
        tmc_langs::SubmissionStatus,
        // submit
        // waitForSubmission
        tmc_langs::SubmissionFinished,
        tmc_langs::TestCase,
        tmc_langs::SubmissionFeedbackQuestion,
        tmc_langs::SubmissionFeedbackKind,
        // listSettings
        tmc_langs::TmcConfig,
    }
    .unwrap()
}
