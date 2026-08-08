// todo: handle output dir

use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

pub fn create(file_path: &Path, output_parent: Option<&Path>) -> Result<PathBuf, io::Error> {
    // Use CWD if not set
    let output_parent = match output_parent {
        Some(path) => path.to_path_buf(),
        None => env::current_dir()?,
    };

    let basename = file_path.file_stem().unwrap_or_default().to_string_lossy();
    let timestamp = jiff::Zoned::now().strftime("%Y-%m-%d %H-%M-%S");

    // "/path/to/filename.ods" -> "/cwd/filename - 2026-07-12 20-35-53/"
    let output_path = output_parent.join(format!("{basename} - {timestamp}"));

    // Creates output dir error if it already existed
    fs::create_dir_all(output_parent)?;
    fs::create_dir(&output_path)?;

    Ok(output_path)
}
