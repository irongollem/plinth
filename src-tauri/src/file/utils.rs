use std::path::PathBuf;

use crate::error::AppError;

use super::compressors;

pub fn clean_name(name: &str) -> String {
    name.trim().to_lowercase().replace(" ", "_")
}

pub fn calculate_total_size(
    group_and_model_dirs: &[PathBuf],
    files_for_3pk: &[PathBuf],
    files_for_zip: &[PathBuf],
) -> Result<(u32, u32), AppError> {
    let (group_and_model_size, group_and_model_files) =
        compressors::determine_dir_size(group_and_model_dirs)?;
    let (files_for_3pk_size, files_for_3pk_count) = compressors::determine_dir_size(files_for_3pk)?;
    let (files_for_zip_size, files_for_zip_count) = compressors::determine_dir_size(files_for_zip)?;

    let total_size = group_and_model_size + files_for_3pk_size + files_for_zip_size;
    let total_files = group_and_model_files + files_for_3pk_count + files_for_zip_count;

    Ok((total_size, total_files))
}
