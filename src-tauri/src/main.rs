#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod export;
mod models;
mod substack;
mod utils;

use models::{ExportJobRequest, ExportJobResult, PublicationRequest, PublicationResponse};

#[tauri::command]
async fn load_publication_posts(request: PublicationRequest) -> Result<PublicationResponse, String> {
    substack::load_publication_posts(request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn run_export_job(request: ExportJobRequest) -> Result<ExportJobResult, String> {
    export::run_export_job(request)
        .await
        .map_err(|error| error.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![load_publication_posts, run_export_job])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
