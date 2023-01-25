//! CLI client for TMC.

use anyhow::{Context, Result};
use clap::Parser;
use std::{any::Any, fs::File, io::Write, path::PathBuf, process::ExitCode};
use tmc_langs::{notification_reporter, progress_reporter, ClientUpdateData};
use tmc_langs_cli::{
    app::Cli,
    map_parsing_result,
    output::{CliOutput, OutputData, OutputResult, Status, StatusUpdateData},
    ParsingResult,
};

fn main() -> ExitCode {
    env_logger::init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(()) => ExitCode::FAILURE,
    }
}

fn run() -> Result<(), ()> {
    // catch unwind is overkill here as try_parse should never panic, but might as well
    let cli = match std::panic::catch_unwind(Cli::try_parse).map(map_parsing_result) {
        // parsed correctly
        Ok(ParsingResult::Ok(cli)) => cli,
        // called with --help
        Ok(ParsingResult::Help(e)) => {
            // --help was probably called by a human, so we'll just print the human-readable help
            println!("{e}");
            return Ok(());
        }
        // failed to parse
        Ok(ParsingResult::Err(output)) => {
            print_output(&output, false).expect("should never fail");
            return Err(());
        }
        // panicked
        Err(err) => {
            // pretty = false to be safe
            print_panic(err, false);
            return Err(());
        }
    };
    let pretty = cli.pretty;
    match std::panic::catch_unwind(|| {
        register_reporters(pretty);
        tmc_langs_cli::run(cli)
    }) {
        Ok(Ok(output)) => {
            print_output(&output, pretty).map_err(|_| ())?;
            Ok(())
        }
        Ok(Err(printable)) => {
            print_output_with_file(&printable.output, pretty, printable.sandbox_path)
                .map_err(|_| ())?;
            Err(())
        }
        Err(err) => {
            print_panic(err, pretty);
            Err(())
        }
    }
}

fn register_reporters(pretty: bool) {
    notification_reporter::init(Box::new(move |warning| {
        let warning_output = CliOutput::Notification(warning);
        if let Err(err) = print_output(&warning_output, pretty) {
            log::error!("printing warning failed: {}", err);
        }
    }));
    progress_reporter::subscribe::<(), _>(move |update| {
        let output = CliOutput::StatusUpdate(StatusUpdateData::None(update));
        let _r = print_output(&output, pretty);
    });
    progress_reporter::subscribe::<ClientUpdateData, _>(move |update| {
        let output = CliOutput::StatusUpdate(StatusUpdateData::ClientUpdateData(update));
        let _r = print_output(&output, pretty);
    });
}

fn print_panic(err: Box<dyn Any + Send>, pretty: bool) {
    // currently only prints a message if the panic is called with str or String; this should be good enough
    let error_message = if let Some(string) = err.downcast_ref::<&str>() {
        format!("Process panicked unexpectedly with message: {string}")
    } else if let Some(string) = err.downcast_ref::<String>() {
        format!("Process panicked unexpectedly with message: {string}")
    } else {
        "Process panicked unexpectedly without an error message".to_string()
    };
    let output = CliOutput::OutputData(Box::new(OutputData {
        status: Status::Crashed,
        message: error_message,
        result: OutputResult::Error,
        data: None,
    }));
    print_output(&output, pretty).expect("should never fail");
}

fn print_output(output: &CliOutput, pretty: bool) -> Result<()> {
    print_output_with_file(output, pretty, None)
}

fn print_output_with_file(output: &CliOutput, pretty: bool, path: Option<PathBuf>) -> Result<()> {
    let result = if pretty {
        serde_json::to_string_pretty(&output)
    } else {
        serde_json::to_string(&output)
    }
    .with_context(|| format!("Failed to convert {output:?} to JSON"))?;
    println!("{result}");

    if let Some(path) = path {
        let mut file = File::create(&path)
            .with_context(|| format!("Failed to open file at {}", path.display()))?;
        file.write_all(result.as_bytes())
            .with_context(|| format!("Failed to write result to {}", path.display()))?;
    }
    Ok(())
}
