#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use flying_carpet_core::{
    bluetooth, clean_up_transfer, network, start_transfer, utils, Transfer, WiFiInterface, UI,
};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::{fs, sync::Mutex};
use tauri::{Emitter, State, Window};
use tokio;
use tokio::sync::mpsc;

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
    fn show_pin(&self, pin: &str) {
        println!("showing pin");
        self.window
            .lock()
            .expect("Couldn't lock GUI mutex")
            .emit(
                "showPin",
                Payload {
                    message: pin.to_string(),
                },
            )
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
        .expect("Couldn't lock state hotspot mutex.");
    let hotspot = &*hotspot;
    let ssid = state.ssid.lock().expect("Couldn't lock state ssid mutex.");
    let ssid = &*ssid;
    match network::stop_hotspot(hotspot.as_ref(), ssid.as_deref()) {
        Err(e) => message += &format!("\nError stopping hotspot: {} \n", e),
        Ok(msg) => message += &format!("\n{}", msg),
    };

    window
        .emit("enableUi", Progress { value: 0 })
        .expect("Couldn't emit to window");
    message
}

#[tauri::command]
fn start_async(
    state: State<Transfer>,
    mode: String,
    peer: Option<String>,
    password: Option<String>,
    interface: WiFiInterface,
    file_list: Option<Vec<String>>,
    receive_dir: Option<String>,
    using_bluetooth: bool,
    window: Window,
) {
    let thread_window = window.clone();
    let gui = GUI {
        window: Arc::new(Mutex::new(thread_window)),
    };

    let transfer_hotspot = state.hotspot.clone();
    let transfer_ssid = state.ssid.clone();

    // used by windows because we have to implement our own UI for PIN confirmation in non-UWP apps.
    // sends the user's choice of whether the bluetooth PINs match to know whether to pair.
    let (ble_ui_tx, ble_ui_rx) = mpsc::channel(1);

    let cancel_handle = tokio::spawn(async move {
        let stream: std::option::Option<tokio::net::TcpStream> = start_transfer(
            mode,
            using_bluetooth,
            peer,
            password,
            interface,
            file_list,
            receive_dir,
            &gui,
            transfer_hotspot.clone(),
            transfer_ssid.clone(),
            ble_ui_rx,
        )
        .await;
        clean_up_transfer(stream, transfer_hotspot, transfer_ssid, &gui).await;
    });
    let mut state_cancel_handle = state.cancel_handle.lock().unwrap();
    *state_cancel_handle = Some(cancel_handle);
    let mut state_ble_ui_tx = state.ble_ui_tx.lock().unwrap();
    *state_ble_ui_tx = Some(ble_ui_tx);
}

#[tokio::main]
async fn main() {
    tauri::async_runtime::set(tokio::runtime::Handle::current());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .manage(Transfer::new())
        .invoke_handler(tauri::generate_handler![
            start_async,
            cancel_transfer,
            is_dir,
            expand_files,
            generate_password,
            get_wifi_interfaces,
            check_support,
            user_bluetooth_pair,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// for javascript, None/null means no error and Some(String) means error message
#[tauri::command]
async fn check_support() -> Option<String> {
    bluetooth::check_support()
        .await
        .map_err(|e| e.to_string())
        .err()
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

#[tauri::command]
fn get_wifi_interfaces() -> Vec<WiFiInterface> {
    match network::get_wifi_interfaces() {
        Ok(interfaces) => interfaces,
        Err(_e) => vec![], // if there was an error, just return empty list of interfaces and let javascript detect "no wifi card found"
    }
}

#[tauri::command]
fn user_bluetooth_pair(choice: bool, state: State<Transfer>) {
    println!("in user_bluetooth_pair");
    let ble_ui_tx = state
        .ble_ui_tx
        .lock()
        .expect("Could not lock ble_ui_tx mutex");
    let ble_ui_tx = ble_ui_tx.as_ref().expect("State ble_ui_tx was None");
    let ble_ui_tx = ble_ui_tx.clone();

    tokio::spawn(async move {
        ble_ui_tx
            .send(choice)
            .await
            .expect("Could not send on ble_ui_tx");
        println!("sent in user_bluetooth_pair");
    });
}
