use rand::Rng;
use sha2::{Digest, Sha256};
use std::{
    ffi::{c_char, CString},
    fs, io,
    path::{Path, PathBuf},
    process,
};

use crate::{FCError, MAJOR_VERSION};

#[derive(Debug, PartialEq)]
pub enum BluetoothMessage {
    Pin(String),
    PairApproved,
    PairSuccess,
    PairFailure,
    AlreadyPaired,
    UserCanceled,
    StartedAdvertising,
    PeerOS(String),
    SSID(String),
    Password(String),
    PeerReadSsid,
    PeerReadPassword,
    OtherError(String),
}

unsafe impl Send for BluetoothMessage {}
unsafe impl Sync for BluetoothMessage {}

pub fn run_command(
    program: &str,
    parameters: Option<Vec<&str>>,
) -> std::io::Result<process::Output> {
    match parameters {
        Some(p) => process::Command::new(program).args(p).output(),
        None => process::Command::new(program).output(),
    }
}

pub fn expand_dir(dir: PathBuf) -> (Vec<String>, Vec<PathBuf>) {
    let mut files_found = vec![];
    let mut dirs_to_search = vec![];
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    dirs_to_search.push(entry.path());
                }
                if metadata.is_file() {
                    files_found.push(entry.path().to_string_lossy().to_string());
                }
            }
        }
    }
    (files_found, dirs_to_search)
}

pub fn make_parent_directories(full_path: &Path) -> io::Result<()> {
    if let Some(dirs) = full_path.parent() {
        fs::create_dir_all(dirs)?;
    }
    Ok(())
}

pub fn get_key_and_ssid(password: &str) -> ([u8; 32], String) {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let key = hasher.finalize();
    let ssid = format!("flyingCarpet_{:02x}{:02x}", key[0], key[1]);
    (key.into(), ssid)
}

pub fn hash_file(filename: &Path) -> Result<Vec<u8>, FCError> {
    let mut file = fs::File::open(filename)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_vec())
}

pub fn generate_password() -> String {
    let mut rng = rand::thread_rng();
    let chars: Vec<char> = "23456789abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ"
        .chars()
        .collect();
    const PASSWORD_LENGTH: usize = 8;
    let mut password: Vec<char> = vec!['\0'; PASSWORD_LENGTH];
    for i in 0..PASSWORD_LENGTH {
        let current_char_index = rng.gen_range(0..chars.len());
        password[i] = chars[current_char_index];
    }
    String::from_iter(password)
}

pub fn make_size_readable(size: u64) -> String {
    let size = size as f64;
    const KB: f64 = 1000.0;
    const MB: f64 = KB * 1000.0;
    const GB: f64 = MB * 1000.0;
    if size < KB {
        format!("{} bytes", size)
    } else if size < MB {
        format!("{:.2}KB", size / KB)
    } else if size < GB {
        format!("{:.2}MB", size / MB)
    } else {
        format!("{:.2}GB", size / GB)
    }
}

pub fn format_time(seconds: f64) -> String {
    if seconds > 60.0 {
        let minutes = seconds as u64 / 60;
        let seconds = seconds % 60.0;
        format!("{} minutes {:.2} seconds", minutes, seconds)
    } else {
        format!("{:.2} seconds", seconds)
    }
}

pub fn is_compatible(peer_version: u64) -> bool {
    // version 8 is not compatible with previous versions
    // TODO: test 8/9
    peer_version == MAJOR_VERSION
}

#[cfg(test)]
mod tests {
    use crate::utils::make_size_readable;

    #[test]
    fn size_readable() {
        assert_eq!(&make_size_readable(999), "999 bytes");
        assert_eq!(&make_size_readable(198_213), "198.21KB");
        assert_eq!(&make_size_readable(48_732_394), "48.73MB");
        assert_eq!(&make_size_readable(8_273_591_032), "8.27GB");
    }

    #[test]
    fn utf8_ok() {
        match super::run_command("ipconfig", None) {
            Ok(output) => {
                let stdout = output.stdout;
                let string = match String::from_utf8(stdout.clone()) {
                    Ok(s) => s,
                    Err(e) => panic!("{}", e),
                };
                print!("stdout: ");
                for byte in stdout {
                    print!("{:02x} ", byte);
                }
                print!("\n");
                println!("string: {}", string);
            }
            Err(e) => println!("{}", e),
        }
    }
}

pub fn rust_to_c_string(s: &str) -> *const c_char {
    CString::new(s).unwrap().into_raw()
}
