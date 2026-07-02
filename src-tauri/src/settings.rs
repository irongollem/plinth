use crate::models::{CompressionType, Settings};
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};
use sysinfo::System;
use tauri::{AppHandle, Wry};
use tauri_plugin_store::{Store, StoreExt as _};

pub(crate) static SETTINGS_CACHE: Lazy<Mutex<Settings>> = Lazy::new(|| {
    Mutex::new(Settings {
        scratch_dir: None,
        target_dir: None,
        compression_type: None,
        chunk_size: None,
        max_compression_threads: None,
    })
});

const STORE_PATH: &str = "settings.json";

async fn get_store_arc(app_handle: &AppHandle) -> Result<Arc<Store<Wry>>, String> {
    let store_res = app_handle.get_store(STORE_PATH);
    match store_res {
        Some(store) => Ok(store),
        None => app_handle.store(STORE_PATH).map_err(|err| err.to_string()),
    }
}
#[tauri::command]
#[specta::specta]
pub async fn get_settings(app_handle: AppHandle) -> Result<Settings, String> {
    let store = get_store_arc(&app_handle)
        .await
        .map_err(|_| "Failed to get store".to_string())?;

    let scratch_dir = store
        .get("scratch_dir")
        .and_then(|v| v.as_str().map(String::from));

    let target_dir = store
        .get("target_dir")
        .and_then(|v| v.as_str().map(String::from));

    let chunk_size = store
        .get("chunk_size")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let max_compression_threads = store
        .get("max_compression_threads")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let settings = Settings {
        scratch_dir,
        target_dir,
        compression_type: Some(CompressionType::Zip),
        chunk_size,
        max_compression_threads,
    };

    {
        let mut cache = SETTINGS_CACHE
            .lock()
            .map_err(|e| format!("Failed to get cache: {}", e))?;
        *cache = settings.clone();
    }

    Ok(settings)
}

#[tauri::command]
#[specta::specta]
pub async fn set_settings(app_handle: AppHandle, settings: Settings) -> Result<(), String> {
    {
        let mut cache = SETTINGS_CACHE
            .lock()
            .map_err(|e| format!("Failed to lock cache: {}", e))?;
        *cache = settings.clone();
    }

    let store = get_store_arc(&app_handle)
        .await
        .map_err(|e| e.to_string())?;

    // Save each setting individually to match your existing pattern
    if let Some(dir) = &settings.scratch_dir {
        store.set("scratch_dir", dir.to_string());
    }
    if let Some(dir) = &settings.target_dir {
        store.set("target_dir", dir.to_string());
    }
    if let Some(compression) = &settings.compression_type {
        let compression_str = match compression {
            CompressionType::Zip => "Zip",
            CompressionType::SevenZip => "7zip",
        };
        store.set("compression_type", compression_str);
    }
    if let Some(max_threads) = settings.max_compression_threads {
        store.set("max_compression_threads", max_threads);
    }

    store.save().map_err(|e| e.to_string())
}

pub fn get_optimal_thread_count() -> u32 {
    let settings_result = SETTINGS_CACHE.lock();
    if let Ok(settings) = settings_result {
        if let Some(thread_count) = settings.max_compression_threads {
            return std::cmp::max(1, thread_count);
        }
    }
    let sys = System::new_all();
    let cpu_count = sys.cpus().len() as u32;
    std::cmp::max(1, cpu_count.saturating_sub(1))
}
