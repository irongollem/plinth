mod catalog;
mod error;
mod file;
mod image;
mod models;
mod render;
mod settings;

use catalog::commands::{
    add_catalog_tag, cancel_catalog_job, get_catalog_model_files, get_catalog_releases,
    get_catalog_stats, get_catalog_tags, get_duplicate_groups, remove_catalog_tag, search_catalog,
    start_catalog_scan, start_duplicate_scan,
};
use file::commands::{add_model, cancel_compression, create_release, finalize_release};
use models::events::{CompressionStatus, DuplicateStatus, RenderStatus, ScanStatus};
use render::commands::{cancel_render, detect_blender, start_render};
use std::env;
use std::sync::Mutex;
use tauri::{Emitter, Listener, Manager};
#[allow(unused_imports)]
use tauri_plugin_fs::FsExt;
use tauri_specta::{collect_commands, collect_events, Builder};

#[cfg(debug_assertions)]
use specta_typescript::Typescript;

/// A .3dpak path passed on the command line (file association / double-click).
/// The startup emit fires before the webview has registered any listener and
/// Tauri events are not queued, so the path is parked here for the frontend
/// to fetch once it has mounted.
pub struct PendingPackageOpen(Mutex<Option<String>>);

#[tauri::command]
#[specta::specta]
fn get_pending_3dpak(state: tauri::State<'_, PendingPackageOpen>) -> Option<String> {
    state.0.lock().ok().and_then(|mut pending| pending.take())
}

/// One builder feeds both the invoke handler and (in debug) the TypeScript
/// bindings export, so the command/event lists can't drift apart.
fn create_specta_builder() -> Builder {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            add_model,
            create_release,
            finalize_release,
            cancel_compression,
            settings::get_settings,
            settings::set_settings,
            detect_blender,
            start_render,
            cancel_render,
            get_pending_3dpak,
            start_catalog_scan,
            start_duplicate_scan,
            cancel_catalog_job,
            search_catalog,
            get_catalog_tags,
            add_catalog_tag,
            remove_catalog_tag,
            get_catalog_model_files,
            get_catalog_stats,
            get_duplicate_groups,
            get_catalog_releases,
        ])
        .events(collect_events![
            CompressionStatus,
            RenderStatus,
            ScanStatus,
            DuplicateStatus,
        ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = env::args().collect();
    let maybe_3dpak_path = if args.len() > 1 {
        let file_path = &args[1];
        if file_path.ends_with(".3dpak") || file_path.ends_with(".3pk") {
            Some(file_path.clone())
        } else {
            None
        }
    } else {
        None
    };

    let builder = create_specta_builder();

    #[cfg(debug_assertions)]
    builder
        .export(
            Typescript::default()
                .formatter(specta_typescript::formatter::biome)
                .header("// @ts-nocheck\n// eslint-disable\n// biome-ignore lint/*: auto-generated file\n"),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");

    tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let app_handle = app.handle().clone();

            app.manage(PendingPackageOpen(Mutex::new(maybe_3dpak_path)));

            let drag_drop_handle = app_handle.clone();
            app_handle.listen("tauri://drag-drop", move |event| {
                if let Ok(payload_json) = serde_json::from_str::<serde_json::Value>(event.payload())
                {
                    if let Some(paths) = payload_json.get("paths").and_then(|p| p.as_array()) {
                        for path_value in paths {
                            if let Some(path_str) = path_value.as_str() {
                                if path_str.ends_with(".3dpak") || path_str.ends_with(".3pk") {
                                    let _ = drag_drop_handle.emit("3dpak-open", path_str);
                                }
                            }
                        }
                    }
                }
            });

            tauri::async_runtime::spawn(async move {
                match settings::get_settings(app_handle).await {
                    Ok(settings) => println!("Settings loaded succesfully: {:?}", settings),
                    Err(err) => eprintln!("Failed to load settings: {}", err),
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
