use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use ods_to_tsv::{Error, OdsFile, TsvWriter, ValueMode, create_export_dir};

/// Extracts each sheet of an OpenDocument Spreadsheet into a separate .tsv
/// file, in a timestamped directory created in the current working directory.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The spreadsheet to extract (.ods or .fods)
    file: PathBuf,

    /// Extract the raw stored values (ISO dates, unlocalized numbers)
    /// instead of the formatted display text
    #[arg(short, long)]
    raw: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            exit_code(&e)
        }
    }
}

/// Maps error categories to exit codes; this is a documented CLI contract
/// (see the README). Clap owns 2 for usage errors, so custom codes start
/// at 3.
fn exit_code(e: &Error) -> ExitCode {
    match e {
        Error::NotSpreadsheet(_) | Error::MissingContent | Error::UnrecognizedFormat => {
            ExitCode::from(3)
        }
        Error::Zip(_) | Error::Xml(_) => ExitCode::from(4),
        Error::Encrypted => ExitCode::from(5),
        Error::Io(_) => ExitCode::FAILURE,
    }
}

fn run(args: &Args) -> Result<(), Error> {
    // Validate the input before creating anything on disk.
    let mut ods = OdsFile::open(&args.file)?;

    // An empty parent puts the export directory in the CWD.
    let output_dir = create_export_dir(&args.file, Path::new(""))?;
    println!("Exporting to directory: {}", output_dir.display());

    let mode = if args.raw {
        ValueMode::Raw
    } else {
        ValueMode::Formatted
    };
    let mut writer = TsvWriter::new(&output_dir);
    for sheet in ods.sheets(mode)? {
        let sheet = sheet?;
        println!("  Extracting {}...", sheet.name);
        writer.write(&sheet)?;
    }
    Ok(())
}
