//! CLI client for TMC

#[quit::main]
fn main() {
    env_logger::init();
    tmc_langs_cli::run()
}
