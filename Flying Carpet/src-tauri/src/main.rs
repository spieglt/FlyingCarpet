#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use flying_carpet_core::{
    clean_up_transfer, network, start_transfer, utils, PeerResource, Transfer, UI,
};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::{fs, sync::Mutex};
use tauri::{State, Window};
use tokio;

#[derive(Clone, serde::Serialize)]
struct Payload {
    message: String,
}

#[derive(Clone, serde::Serialize)]
struct Progress {
    value: u8,
}

#[derive(Clone)]
struct GUI {
    window: Arc<Mutex<Window>>,
}

impl UI for GUI {
    fn output(&self, msg: &str) {
        self.window
            .lock()
            .expect("Couldn't lock GUI mutex")
            .emit(
                "outputMsg",
                Payload {
                    message: msg.to_string(),
                },
            )
            .expect("could not emit event");
    }
    fn show_progress_bar(&self) {
        self.window
            .lock()
            .expect("Couldn't lock GUI mutex")
            .emit("showProgressBar", Progress { value: 0 })
            .expect("could not emit event");
    }
    fn update_progress_bar(&self, percent: u8) {
        self.window
            .lock()
            .expect("Couldn't lock GUI mutex")
            .emit("updateProgressBar", Progress { value: percent })
            .expect("could not emit event");
    }
    fn enable_ui(&self) {
        self.window
            .lock()
            .expect("Couldn't lock GUI mutex")
            .emit("enableUi", Progress { value: 0 })
            .expect("could not emit event");
    }
}

#[tauri::command]
fn cancel_transfer(window: Window, state: State<Transfer>) -> String {
    let mut message = String::new();

    // cancel file transfer, which should close tcp socket?
    let cancel_handle = &mut state.cancel_handle.lock().unwrap();
    if let Some(handle) = cancel_handle.as_ref() {
        handle.abort();
        while !handle.is_finished() {
            println!("Waiting for transfer to cancel...");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        **cancel_handle = None;
        message += "Transfer cancelled"
    } else {
        message += "No transfer to cancel"
    }

    // shut down hotspot
    let hotspot = state
        .hotspot
        .lock()
        .expect("Couldn't lock outer state hotspot mutex.");
    let hotspot = hotspot
        .lock()
        .expect("Couldn't lock inner state hotspot mutex.");
    if let Some(hotspot) = &*hotspot {
        match network::stop_hotspot(&hotspot) {
            Err(e) => message += &format!("Error stopping hotspot: {} \n", e),
            _ => (),
        };
    }

    window
        .emit("enableUi", Progress { value: 0 })
        .expect("Couldn't emit to window");
    message
}

#[tauri::command]
fn start_async(
    state: State<Transfer>,
    mode: String,
    peer: String,
    password: String,
    ssid: Option<String>,
    file_list: Option<Vec<String>>,
    receive_dir: Option<String>,
    window: Window,
) {
    let thread_window = window.clone();
    let gui = GUI {
        window: Arc::new(Mutex::new(thread_window)),
    };

    // let tauri_hotspot = Arc::new(Mutex::<Option<PeerResource>>::new(None));
    // let transfer_hotspot = tauri_hotspot.clone();
    // let mut tauri_hotspot_mutex = state.hotspot.lock().expect("Couldn't lock outer hotspot mutex.");
    // *tauri_hotspot_mutex = tauri_hotspot;

    let hotspot = state.hotspot.lock().expect("Couldn't lock hotspot mutex.");
    let transfer_hotspot: std::sync::Arc<std::sync::Mutex<Option<PeerResource>>> =
        (*hotspot).clone();
    let cleanup_hotspot = transfer_hotspot.clone();

    let cancel_handle = tokio::spawn(async move {
        let stream = start_transfer(
            mode,
            peer,
            password,
            ssid,
            file_list,
            receive_dir,
            &gui,
            transfer_hotspot,
        )
        .await;
        clean_up_transfer(stream, cleanup_hotspot, &gui).await;
    });
    let mut state_cancel_handle = state.cancel_handle.lock().unwrap();
    *state_cancel_handle = Some(cancel_handle);
}

#[tokio::main]
async fn main() {
    tauri::async_runtime::set(tokio::runtime::Handle::current());
    tauri::Builder::default()
        .manage(Transfer::new())
        .invoke_handler(tauri::generate_handler![
            start_async,
            cancel_transfer,
            is_dir,
            expand_files,
            generate_password,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
fn is_dir(path: &str) -> bool {
    match fs::metadata(path) {
        Ok(m) => m.is_dir(),
        Err(_) => false,
    }
}

#[tauri::command]
fn expand_files(paths: Vec<&str>) -> Vec<String> {
    let path_bufs: Vec<PathBuf> = paths
        .iter()
        .filter_map(|p| PathBuf::from_str(p).ok())
        .collect();
    let mut files: Vec<String> = vec![];
    let mut dirs_to_search: Vec<PathBuf> = vec![];
    for path in path_bufs {
        if let Some(metadata) = fs::metadata(&path).ok() {
            if metadata.is_dir() {
                dirs_to_search.push(path.clone());
            }
            if metadata.is_file() {
                files.push(path.to_string_lossy().to_string());
            }
        }
    }
    while dirs_to_search.len() > 0 {
        let (mut temp_files, mut temp_dirs) = utils::expand_dir(
            dirs_to_search
                .pop()
                .expect("Had dirs to search but couldn't pop."),
        );
        files.append(&mut temp_files);
        dirs_to_search.append(&mut temp_dirs);
    }
    files
}

#[tauri::command]
fn generate_password() -> String {
    utils::generate_password()
}
