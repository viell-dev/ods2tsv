use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use ods_to_tsv::{Error, OdsFile, TsvWriter, create_export_dir};

/// Extracts each sheet of an OpenDocument Spreadsheet into a separate .tsv
/// file, in a timestamped directory created in the current working directory.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// The spreadsheet to extract (.ods or .fods)
    file: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<(), Error> {
    // Validate the input before creating anything on disk.
    let mut ods = OdsFile::open(&args.file)?;

    // An empty parent puts the export directory in the CWD.
    let output_dir = create_export_dir(&args.file, Path::new(""))?;
    println!("Exporting to directory: {}", output_dir.display());

    let mut writer = TsvWriter::new(&output_dir);
    for sheet in ods.sheets()? {
        let sheet = sheet?;
        println!("  Extracting {}...", sheet.name);
        writer.write(&sheet)?;
    }
    Ok(())
}
