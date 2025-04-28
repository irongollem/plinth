mod error;
mod file;
mod image;
mod models;
mod settings;

use file::commands::{add_model, cancel_compression, create_release, finalize_release};
use models::events::CompressionStatus;
use specta_typescript::Typescript;
use std::env;
use tauri::{Emitter, Listener};
#[allow(unused_imports)]
use tauri_plugin_fs::FsExt;
use tauri_specta::{collect_commands, collect_events, Builder};

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

    #[cfg(debug_assertions)]
    export_typescript_bindings();

    tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let app_handle = app.handle().clone();

            if let Some(file_path) = maybe_3dpak_path {
                app_handle
                    .emit("3dpak-open", file_path)
                    .unwrap_or_else(|e| {
                        eprintln!("Failed to emit 3dpak-open event: {}", e);
                    });
            }

            let drag_drop_handle = app_handle.clone();
            app_handle.listen("tauri://drag-drop", move |event| {
                if let Ok(payload_json) =
                    serde_json::from_str::<serde_json::Value>(&event.payload())
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
        .invoke_handler(tauri::generate_handler![
            add_model,
            create_release,
            finalize_release,
            settings::get_settings,
            settings::set_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(debug_assertions)]
fn export_typescript_bindings() {
    let builder = Builder::<tauri::Wry>::new();
    builder
        .commands(collect_commands![
            add_model,
            create_release,
            finalize_release,
            cancel_compression,
            settings::get_settings,
            settings::set_settings,
        ])
        .events(collect_events![CompressionStatus,])
        .export(
            Typescript::default()
                .formatter(specta_typescript::formatter::biome)
                .header(""),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");
}
