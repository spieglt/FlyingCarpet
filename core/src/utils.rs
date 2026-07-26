use rand::Rng;
use sha2::{Digest, Sha256};
use std::{
    ffi::{c_char, CString},
    fs, io,
    path::{Path, PathBuf},
    process,
};

use crate::FCError;

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

/// Async twin of [`run_command`], for commands run while a transfer is in flight. Awaiting
/// the child is a cancellation point, so aborting the transfer task lands here instead of
/// after the command returns — `nmcli con up` alone can sit for the better part of a minute.
///
/// `kill_on_drop` means the child is signalled when the task is aborted rather than left
/// running. Note what that does and doesn't buy: nmcli is a D-Bus client, so killing it does
/// not call off the work NetworkManager is already doing on its behalf. Undoing a half-made
/// connection is still `stop_hotspot`'s job, which the cancel path runs afterwards.
pub async fn run_command_async(
    program: &str,
    parameters: Option<Vec<&str>>,
) -> std::io::Result<process::Output> {
    let mut command = tokio::process::Command::new(program);
    command.kill_on_drop(true);
    if let Some(p) = parameters {
        command.args(p);
    }
    command.output().await
}

pub fn expand_dir(dir: PathBuf) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files_found = vec![];
    let mut dirs_to_search = vec![];
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_dir() {
                    dirs_to_search.push(entry.path());
                }
                if metadata.is_file() {
                    files_found.push(entry.path());
                }
            }
        }
    }
    (files_found, dirs_to_search)
}

// Turn what the user picked (files, folders, or a mix — from a dialog or a drag-and-drop)
// into the flat list of files to send, each carrying the relative name the peer will store
// it under.
//
// The rule, matching the Apple and Android senders: *every top-level selection is named
// relative to its own parent directory*. So a selected folder keeps its own name as the
// first component and the receiver recreates it with the files inside, while individually
// selected files arrive flat in the destination. Because each selection is resolved against
// its own parent, selections spanning different directories work too — previously they made
// the sender abort with a strip-prefix error. See docs/send-folder-behavior.md.
pub fn expand_selection(roots: Vec<PathBuf>) -> Vec<crate::SendFile> {
    let mut selected = vec![];
    for root in roots {
        // A root at a filesystem root ("/", "C:\") has no parent to strip; naming its
        // contents relative to itself is the only sane reading, and keeps the relative
        // name from starting with a separator.
        let prefix = root.parent().unwrap_or(&root).to_path_buf();
        let Ok(metadata) = fs::metadata(&root) else {
            continue;
        };
        if metadata.is_file() {
            selected.extend(make_send_file(&root, &prefix));
        } else if metadata.is_dir() {
            let mut dirs_to_search = vec![root];
            while let Some(dir) = dirs_to_search.pop() {
                let (files, subdirs) = expand_dir(dir);
                selected.extend(files.iter().filter_map(|f| make_send_file(f, &prefix)));
                dirs_to_search.extend(subdirs);
            }
        }
    }
    selected
}

// Name `path` relative to `prefix`, normalized to the "/" separators the wire format uses.
// Anything that doesn't sit under the prefix is dropped rather than sent under a wrong (or
// absolute) name.
fn make_send_file(path: &Path, prefix: &Path) -> Option<crate::SendFile> {
    let relative = path.strip_prefix(prefix).ok()?;
    let name = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    if name.is_empty() {
        return None;
    }
    Some(crate::SendFile {
        path: path.to_path_buf(),
        name,
    })
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    // Lay out a temp tree and return its root:
    //   <root>/loose.txt
    //   <root>/Photos/a.jpg
    //   <root>/Photos/sub/b.jpg
    //   <root>/OnlySubdirs/x/1.txt
    //   <root>/OnlySubdirs/y/2.txt
    //   <root>/Other/c.jpg
    fn make_tree(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fc_selection_{}_{}_{:?}",
            std::process::id(),
            tag,
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        for dir in [
            root.join("Photos").join("sub"),
            root.join("OnlySubdirs").join("x"),
            root.join("OnlySubdirs").join("y"),
            root.join("Other"),
        ] {
            fs::create_dir_all(dir).unwrap();
        }
        for (path, contents) in [
            (root.join("loose.txt"), "loose"),
            (root.join("Photos").join("a.jpg"), "a"),
            (root.join("Photos").join("sub").join("b.jpg"), "b"),
            (root.join("OnlySubdirs").join("x").join("1.txt"), "1"),
            (root.join("OnlySubdirs").join("y").join("2.txt"), "2"),
            (root.join("Other").join("c.jpg"), "c"),
        ] {
            fs::write(path, contents).unwrap();
        }
        root
    }

    fn names(roots: Vec<PathBuf>) -> Vec<String> {
        let mut names: Vec<String> = expand_selection(roots)
            .into_iter()
            .map(|f| f.name)
            .collect();
        names.sort();
        names
    }

    // The headline behavior: selecting a folder recreates that folder on the receiving end
    // with everything inside it, rather than dumping its contents into the destination.
    #[test]
    fn selected_folder_keeps_its_own_name() {
        let root = make_tree("folder");
        assert_eq!(
            names(vec![root.join("Photos")]),
            vec!["Photos/a.jpg", "Photos/sub/b.jpg"]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    // Individually selected files still land flat in the destination.
    #[test]
    fn selected_files_are_flat() {
        let root = make_tree("files");
        assert_eq!(
            names(vec![
                root.join("loose.txt"),
                root.join("Photos").join("a.jpg")
            ]),
            vec!["a.jpg", "loose.txt"]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    // A folder whose top level holds only subdirectories used to lose a level of structure
    // (single subdir) or abort the transfer with a strip-prefix error (sibling subdirs),
    // because the prefix was "whichever parent had the fewest components".
    #[test]
    fn folder_of_only_subdirectories_keeps_full_structure() {
        let root = make_tree("subdirs");
        assert_eq!(
            names(vec![root.join("OnlySubdirs")]),
            vec!["OnlySubdirs/x/1.txt", "OnlySubdirs/y/2.txt"]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    // Dropping two folders at once, or files from different directories, is resolved
    // per-selection now; both used to fail.
    #[test]
    fn selections_from_different_directories_coexist() {
        let root = make_tree("mixed");
        assert_eq!(
            names(vec![root.join("Photos"), root.join("Other")]),
            vec!["Other/c.jpg", "Photos/a.jpg", "Photos/sub/b.jpg"]
        );
        assert_eq!(
            names(vec![
                root.join("Photos").join("a.jpg"),
                root.join("Other").join("c.jpg"),
            ]),
            vec!["a.jpg", "c.jpg"]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    // Relative names must never start with a separator: the Rust receiver rejects those
    // outright (Component::RootDir in receiving::sanitize_relative_filename), which is
    // exactly how Android's folder sends used to fail against desktop peers.
    #[test]
    fn names_are_relative_and_slash_separated() {
        let root = make_tree("separators");
        for file in expand_selection(vec![root.join("Photos"), root.join("loose.txt")]) {
            assert!(
                !file.name.starts_with('/'),
                "leading slash in {}",
                file.name
            );
            assert!(!file.name.contains('\\'), "backslash in {}", file.name);
            assert!(
                crate::receiving::sanitize_relative_filename(&file.name).is_ok(),
                "receiver rejected {}",
                file.name
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nonexistent_selection_is_skipped() {
        assert!(expand_selection(vec![PathBuf::from("fc_no_such_path_here")]).is_empty());
    }
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

pub fn compute_hmac(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).unwrap();
    mac.update(data);
    mac.finalize().into_bytes().into()
}

pub fn verify_hmac(key: &[u8; 32], data: &[u8], expected: &[u8; 32]) -> bool {
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<sha2::Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).unwrap();
    mac.update(data);
    mac.verify_slice(expected).is_ok()
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
    // 10 chars over the 57-symbol set ≈ 2^58: makes a precomputed PBKDF2 table over the
    // whole password space (possible because the PSK salt is a fixed domain string)
    // infeasible in both compute and storage. Must match Android and Apple.
    const PASSWORD_LENGTH: usize = 10;
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
    // v10 (shared network mode and the new protocol) is a clean break from earlier
    // versions. If transferring with a higher version, that version decides compatibility.
    peer_version >= 10
}

#[cfg(test)]
mod tests {
    use crate::utils::make_size_readable;

    // The whole point of run_command_async: a transfer that's waiting on an external command
    // can still be cancelled. Aborting the task has to land while the child is running, not
    // after it exits — with the blocking run_command this takes the full five seconds.
    // Unix-only because the test needs a slow command that exists; run_command_async is only
    // used by the Linux network code.
    #[cfg(unix)]
    #[tokio::test]
    async fn async_command_is_interruptible() {
        let handle =
            tokio::spawn(async { super::run_command_async("sleep", Some(vec!["5"])).await });
        // let it get as far as actually spawning the child
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let abort_issued = std::time::Instant::now();
        handle.abort();
        let outcome = handle.await;
        let waited = abort_issued.elapsed();

        assert!(outcome.unwrap_err().is_cancelled());
        assert!(
            waited < std::time::Duration::from_secs(1),
            "cancel took {:?}; it should not have waited for the command to finish",
            waited
        );
    }

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
