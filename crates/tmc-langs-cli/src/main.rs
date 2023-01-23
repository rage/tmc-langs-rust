//! CLI client for TMC.

use std::process::ExitCode;

fn main() -> ExitCode {
    env_logger::init();
    match tmc_langs_cli::run() {
        Ok(_) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
