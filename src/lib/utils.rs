use serde;
use serde_json;
use std::{fmt, fs, io};
use std::error::Error;
use std::io::Write;
use std::path::Path;

/// Errors that might occur when saving a file.
#[derive(Debug)]
pub enum FileError {
    Io(io::Error),
    Json(serde_json::Error),
}

impl From<io::Error> for FileError {
    fn from(err: io::Error) -> Self {
        FileError::Io(err)
    }
}

impl From<serde_json::Error> for FileError {
    fn from(err: serde_json::Error) -> Self {
        FileError::Json(err)
    }
}

impl Error for FileError {
    fn description(&self) -> &str {
        match *self {
            FileError::Io(ref err) => err.description(),
            FileError::Json(ref err) => err.description(),
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Saves the file to a temporary file before removing the original to reduce the chance of losing
/// data in the case that something goes wrong during saving.
pub fn safe_file_save(path: &Path, content: &[u8]) -> io::Result<()> {
    let temp_path = path.with_extension("tmp");

    // If the temp file exists, remove it.
    if temp_path.exists() {
        fs::remove_file(&temp_path)?;
    }

    // Create the directory if it doesn't exist.
    if let Some(directory) = path.parent() {
        if !directory.exists() {
            fs::create_dir_all(&temp_path)?;
        }
    }

    // Write the temp file.
    let mut file = fs::File::create(&temp_path)?;
    file.write(content)?;

    // If there's already a file at `path`, remove it.
    if path.exists() {
        fs::remove_file(&path)?;
    }

    // Rename the temp file to the original path name.
    fs::rename(temp_path, path)?;

    Ok(())
}

/// A generic function for safely saving a serializable type to a JSON file.
pub fn save_to_json<T>(json_path: &Path, t: &T) -> Result<(), FileError>
where
    T: serde::Serialize,
{
    let string = serde_json::to_string_pretty(t)?;
    safe_file_save(json_path, string.as_bytes())?;
    Ok(())
}

/// A generic function for safely saving a serializable type to a JSON file.
///
/// **Panics** if an error occurs while attempting to serialize or save the file.
pub fn save_to_json_or_panic<T>(json_path: &Path, t: &T)
where
    T: serde::Serialize,
{
    save_to_json(json_path, t)
        .unwrap_or_else(|err| panic!("failed to save file \"{}\": {}", json_path.display(), err))
}

/// A generic funtion for loading a type from a JSON file.
pub fn load_from_json<'a, T>(json_path: &Path) -> Result<T, FileError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let file = fs::File::open(json_path)?;
    let t = serde_json::from_reader(file)?;
    Ok(t)
}

/// A generic function for loading a type from a json file.
///
/// If deserialization of file loading fails, a `Default` instance will be returned.
pub fn load_from_json_or_default<'a, T>(json_path: &Path) -> T
where
    T: for<'de> serde::Deserialize<'de> + Default,
{
    load_from_json(json_path).unwrap_or_else(|_| Default::default())
}
