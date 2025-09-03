use tauri::{async_runtime::spawn, Manager};
use tokio::sync::Mutex;

use crate::{
    services::{MinimapService, Service},
    state::AppState,
};

mod services;
mod state;

#[tauri::command]
async fn set_window(app: tauri::AppHandle, title: String) -> Result<(), ()> {
    let state = app.state::<Mutex<AppState>>();
    state.lock().await.minimap.set_window(title).await?;
    Ok(())
}

#[tauri::command]
async fn list_window_handles(app: tauri::AppHandle) -> Vec<String> {
    let state = app.state::<Mutex<AppState>>();
    let window_handles = state.lock().await.minimap.list_windows();
    window_handles.unwrap_or_default()
}

#[tauri::command]
async fn stop_services(app: tauri::AppHandle) -> Result<(), ()> {
    let state = app.state::<Mutex<AppState>>();
    log::info!("Stopping services...");
    state.lock().await.minimap.stop().await?;
    Ok(())
}

async fn start_services(app: tauri::AppHandle) -> Result<(), ()> {
    let state = app.state::<Mutex<AppState>>();
    log::info!("Starting services...");
    state.lock().await.minimap.start().await?;
    log::info!("Services started successfully.");
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    platforms::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::new().build())
        .plugin(tauri_plugin_websocket::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_state = Mutex::new(AppState {
                minimap: Box::new(MinimapService::new(app.handle().clone())),
            });
            app.manage(app_state);

            spawn(start_services(app.handle().clone()));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_window_handles,
            stop_services,
            set_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
