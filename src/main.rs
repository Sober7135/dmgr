use std::process::ExitCode;

fn main() -> ExitCode {
    match dmgr::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
