use std::{fs, path::PathBuf};

use crate::error::AppError;

pub fn write_json(json_string: String, path: PathBuf) -> Result<(), AppError> {
    fs::write(path, json_string).map_err(|e| AppError::IoError(e.to_string()))
}
