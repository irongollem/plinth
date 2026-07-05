mod catalog;
mod error;
mod file;
mod image;
mod manifest;
mod models;
mod render;
mod settings;

use catalog::commands::{
    add_catalog_tag, add_group_tag, assign_files_to_pose, batch_move_models, cancel_catalog_job,
    clear_file_pose, combine_catalog_groups, delete_duplicate_files, detach_catalog_group_source,
    get_catalog_designers, get_catalog_group_members, get_catalog_group_sources,
    get_catalog_model_files, get_catalog_releases, get_catalog_stats, get_catalog_tags,
    get_duplicate_groups,
    get_file_variants, merge_duplicate_files, remove_catalog_tag, remove_group_tag,
    rename_catalog_group, search_catalog, search_catalog_groups, set_group_cover,
    set_model_preview, start_catalog_scan, start_duplicate_scan, supports_file_links,
    update_model_metadata,
};
use file::commands::{
    add_model, cancel_compression, create_release, finalize_release, import_release,
    open_with_default_app,
};
use models::events::{CompressionStatus, DuplicateStatus, RenderStatus, ScanStatus};
use render::commands::{
    cancel_render, detect_blender, read_image_base64, start_render, write_png_base64,
};
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
            open_with_default_app,
            import_release,
            settings::get_settings,
            settings::set_settings,
            detect_blender,
            start_render,
            cancel_render,
            read_image_base64,
            write_png_base64,
            get_pending_3dpak,
            start_catalog_scan,
            start_duplicate_scan,
            cancel_catalog_job,
            search_catalog,
            get_catalog_tags,
            add_catalog_tag,
            remove_catalog_tag,
            add_group_tag,
            remove_group_tag,
            get_catalog_model_files,
            get_catalog_stats,
            get_duplicate_groups,
            get_catalog_releases,
            get_catalog_designers,
            update_model_metadata,
            set_model_preview,
            delete_duplicate_files,
            merge_duplicate_files,
            supports_file_links,
            batch_move_models,
            search_catalog_groups,
            get_catalog_group_members,
            get_catalog_group_sources,
            detach_catalog_group_source,
            set_group_cover,
            rename_catalog_group,
            combine_catalog_groups,
            assign_files_to_pose,
            clear_file_pose,
            get_file_variants,
        ])
        .events(collect_events![
            CompressionStatus,
            RenderStatus,
            ScanStatus,
            DuplicateStatus,
        ])
}

/// Shared by the debug-run export and the `bindings_are_current` test, so
/// `cargo test` regenerates src/bindings.ts without launching the app —
/// registering a command in create_specta_builder is all it takes.
#[cfg(debug_assertions)]
fn export_typescript_bindings(builder: &Builder) {
    builder
        .export(
            Typescript::default()
                .formatter(specta_typescript::formatter::biome)
                .header("// @ts-nocheck\n// eslint-disable\n// biome-ignore lint/*: auto-generated file\n"),
            "../src/bindings.ts",
        )
        .expect("Failed to export typescript bindings");
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
    export_typescript_bindings(&builder);

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

#[cfg(test)]
mod tests {
    /// Rewrites src/bindings.ts from the current command list. Tests build
    /// with debug_assertions, so this reuses the exact export the dev app
    /// performs at startup.
    #[test]
    fn bindings_are_current() {
        super::export_typescript_bindings(&super::create_specta_builder());
    }
}
