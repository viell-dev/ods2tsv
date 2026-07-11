use std::env;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("Usage: ods-to-tsv <filename.ods>");
        return ExitCode::from(2);
    };

    // An empty output parent means the export directory lands in the CWD.
    match ods_to_tsv::extract_ods_to_tsv(Path::new(&path), Path::new("")) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
