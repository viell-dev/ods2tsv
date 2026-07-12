use std::env;
use std::path::Path;
use std::process::ExitCode;

use ods_to_tsv::{Error, OdsFile, TsvWriter, create_export_dir};

const USAGE: &str = "Usage: ods-to-tsv <filename.ods>";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let (Some(arg), None) = (args.next(), args.next()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    match arg.to_str() {
        Some("-h" | "--help") => {
            println!("{USAGE}");
            println!();
            println!(
                "Extracts each sheet of an OpenDocument Spreadsheet into a separate .tsv\n\
                 file, in a timestamped directory created in the current working directory."
            );
            return ExitCode::SUCCESS;
        }
        Some("-V" | "--version") => {
            println!("ods-to-tsv {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    match run(Path::new(&arg)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(path: &Path) -> Result<(), Error> {
    // Validate the input before creating anything on disk.
    let mut ods = OdsFile::open(path)?;

    // An empty parent puts the export directory in the CWD.
    let output_dir = create_export_dir(path, Path::new(""))?;
    println!("Exporting to directory: {}", output_dir.display());

    let mut writer = TsvWriter::new(&output_dir);
    for sheet in ods.sheets()? {
        let sheet = sheet?;
        println!("  Extracting {}...", sheet.name);
        writer.write(&sheet)?;
    }
    Ok(())
}
