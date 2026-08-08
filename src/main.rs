use std::{fmt::Debug, path::PathBuf, process::ExitCode};

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Path to the file to export
    #[arg(value_name = "FILE")]
    path: PathBuf,

    /// Export raw values instead of formatted values
    #[arg(short, long)]
    raw: bool,
}

#[expect(clippy::print_stdout, clippy::print_stderr, reason = "CLI")]
fn main() -> ExitCode {
    let args = Args::parse();

    match ods2tsv::run(&args.path, args.raw) {
        Ok(s) => {
            println!("Output path: {s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            e.into()
        }
    }
}
