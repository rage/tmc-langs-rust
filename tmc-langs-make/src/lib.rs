mod check_log;
pub mod error;
pub mod plugin;
pub mod policy;

pub struct CTestCase {
    name: String,
    passed: bool,
    message: String,
    points: Vec<String>,
    valgrind_trace: Option<String>,
    fail_on_valgrind_error: bool,
}
