use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

mod migrations;
mod new_context;
mod problem_details;
mod rls;
mod session;

const USAGE: &str =
    "usage: xtask new-context <name> | xtask check-problem-details | xtask check-migrations";

fn run(args: &[String], out: &mut impl Write) -> Result<bool, String> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("xtask has no parent directory")?;
    match args {
        [command, name] if command == "new-context" => {
            new_context::create(workspace, name)?;
            writeln!(
                out,
                "created bounded context {name} in domain, db, app and api"
            )
            .map_err(|error| format!("cannot write the report: {error}"))?;
            Ok(true)
        }
        [command] if command == "check-problem-details" => problem_details::check(out),
        [command] if command == "check-migrations" => migrations::check(workspace, out),
        _ => Err(USAGE.to_owned()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args, &mut std::io::stdout().lock()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            let reported = writeln!(std::io::stderr().lock(), "{message}");
            ExitCode::from(if reported.is_ok() { 2 } else { 3 })
        }
    }
}
