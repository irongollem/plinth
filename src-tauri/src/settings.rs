use crate::models::{CompressionType, Settings};
use once_cell::sync::Lazy;
use serde_json::json;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Wry};
use tauri_plugin_store::{Store, StoreExt as _};

pub(crate) static SETTINGS_CACHE: Lazy<Mutex<Settings>> = Lazy::new(|| {
    Mutex::new(Settings {
        scratch_dir: None,
        target_dir: None,
        compression_type: None,
        chunk_size: None,
        max_compression_threads: None,
        blender_path: None,
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

fn compression_type_from_str(value: &str) -> CompressionType {
    match value {
        // "7zip" is the legacy spelling older builds wrote to the store
        "SevenZip" | "7zip" => CompressionType::SevenZip,
        _ => CompressionType::Zip,
    }
}

fn compression_type_to_str(value: &CompressionType) -> &'static str {
    match value {
        CompressionType::SevenZip => "SevenZip",
        CompressionType::Zip => "Zip",
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

    let compression_type = store
        .get("compression_type")
        .and_then(|v| v.as_str().map(compression_type_from_str))
        .unwrap_or(CompressionType::Zip);

    let chunk_size = store
        .get("chunk_size")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let max_compression_threads = store
        .get("max_compression_threads")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let blender_path = store
        .get("blender_path")
        .and_then(|v| v.as_str().map(String::from));

    let settings = Settings {
        scratch_dir,
        target_dir,
        compression_type: Some(compression_type),
        chunk_size,
        max_compression_threads,
        blender_path,
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
    let store = get_store_arc(&app_handle)
        .await
        .map_err(|e| e.to_string())?;

    // Some -> write, None -> delete; without the delete a cleared setting
    // would resurrect from disk on the next load
    fn set_or_delete(store: &Store<Wry>, key: &str, value: Option<serde_json::Value>) {
        match value {
            Some(v) => store.set(key, v),
            None => {
                store.delete(key);
            }
        }
    }

    set_or_delete(
        &store,
        "scratch_dir",
        settings.scratch_dir.as_deref().map(|v| json!(v)),
    );
    set_or_delete(
        &store,
        "target_dir",
        settings.target_dir.as_deref().map(|v| json!(v)),
    );
    set_or_delete(
        &store,
        "compression_type",
        settings
            .compression_type
            .as_ref()
            .map(|v| json!(compression_type_to_str(v))),
    );
    set_or_delete(&store, "chunk_size", settings.chunk_size.map(|v| json!(v)));
    set_or_delete(
        &store,
        "max_compression_threads",
        settings.max_compression_threads.map(|v| json!(v)),
    );
    set_or_delete(
        &store,
        "blender_path",
        settings.blender_path.as_deref().map(|v| json!(v)),
    );

    store.save().map_err(|e| e.to_string())?;

    // Update the cache only after the store persisted, so memory and disk
    // can't diverge on a failed save
    let mut cache = SETTINGS_CACHE
        .lock()
        .map_err(|e| format!("Failed to lock cache: {}", e))?;
    *cache = settings;

    Ok(())
}

pub fn get_optimal_thread_count() -> u32 {
    let settings_result = SETTINGS_CACHE.lock();
    if let Ok(settings) = settings_result {
        if let Some(thread_count) = settings.max_compression_threads {
            return std::cmp::max(1, thread_count);
        }
    }
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    std::cmp::max(1, cpu_count.saturating_sub(1))
}
