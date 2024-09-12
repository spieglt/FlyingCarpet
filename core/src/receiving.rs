use crate::{utils, UI};
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit};
use std::{
    error::Error,
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};

pub async fn receive_file<T: UI>(
    folder: &Path,
    key: &[u8],
    stream: &mut TcpStream,
    ui: &T,
    last_file: bool,
) -> Result<(), Box<dyn Error>> {
    let folder = folder.to_owned();
    let cipher = Aes256Gcm::new_from_slice(key)?;
    let start = Instant::now();

    // check destination folder
    fs::read_dir(&folder)?;

    // receive file details
    let (filename, file_size) = receive_file_details(stream).await?;
    // TODO: convert forward slashes to backslashes before receiving if mirroring?
    ui.output(&format!("Filename: {}", filename));
    ui.output(&format!(
        "File size: {}",
        utils::make_size_readable(file_size)
    ));
    let mut bytes_left = file_size;

    // see if we already have the file being sent
    let mut full_path = folder.clone();
    full_path.push(&filename);
    let need_transfer = check_for_file(&full_path, file_size, stream).await?;
    if !need_transfer {
        ui.output("Recipient already has this file, skipping.");
        return Ok(());
    }

    // make parent directories if necessary
    utils::make_parent_directories(&full_path)?;

    // check if file being received already exists. if so, find new filename.
    let mut i = 1;
    while full_path.is_file() {
        let file_name = full_path
            .file_name()
            .expect("could not get filename from full path")
            .to_str()
            .expect("could not convert filename to str");
        let new_name = format!("({}) ", i) + file_name;
        full_path.pop();
        full_path.push(new_name);
        i += 1;
    }

    // open output file
    let mut out_file = fs::File::create(&full_path)?;

    // show progress bar
    ui.show_progress_bar();

    // receive file
    loop {
        tokio::task::yield_now().await;
        let decrypted_bytes = receive_and_decrypt_chunk(&cipher, stream).await?;
        if decrypted_bytes.len() == 0 {
            break;
        }
        bytes_left -= decrypted_bytes.len() as u64;
        out_file.write_all(&decrypted_bytes)?;
        let percent_done = ((file_size - bytes_left) as f64 / file_size as f64) * 100.0;
        ui.update_progress_bar(percent_done as u8);
    }

    // tell sending end we're finished
    stream.write_u64(1).await?;

    // stats
    ui.update_progress_bar(100);
    let output_size = out_file
        .metadata()
        .expect("could not get output file metadata")
        .len();
    let dest_filename = full_path
        .file_name()
        .expect("output file didn't have a name")
        .to_string_lossy();
    ui.output(&format!(
        "Received file {}. Size: {}.",
        dest_filename,
        utils::make_size_readable(output_size)
    ));
    let finish = Instant::now();
    let elapsed = (finish - start).as_secs_f64();
    ui.output(&format!("Receiving took {}", utils::format_time(elapsed)));

    let megabits = 8.0 * (file_size as f64 / 1_000_000.0);
    let mbps = megabits / elapsed;
    ui.output(&format!("Speed: {:.2}mbps", mbps));

    // wait for double confirmation
    if last_file {
        match timeout(Duration::from_secs(2), stream.read_u64()).await {
            Ok(res) => {
                res?;
            }
            Err(_e) => {
                ui.output("Didn't receive confirmation");
            }
        };
    } else {
        let _reply = stream.read_u64().await?;
    }

    Ok(())
}

async fn receive_and_decrypt_chunk(
    cipher: &Aes256Gcm,
    stream: &mut TcpStream,
) -> Result<Vec<u8>, Box<dyn Error>> {
    // receive chunk size
    let chunk_size = stream.read_u64().await? as usize;
    if chunk_size == 0 {
        Ok(vec![])
    } else {
        // receive chunk
        let mut chunk = vec![0u8; chunk_size];
        stream.read_exact(&mut chunk).await?;
        // decrypt
        let nonce = &chunk[..12];
        let ciphertext = &chunk[12..];
        let nonce = aes_gcm::Nonce::from_slice(nonce);
        let decrypted_chunk = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| e.to_string())?;
        Ok(decrypted_chunk)
    }
}

async fn receive_file_details(stream: &mut TcpStream) -> std::io::Result<(String, u64)> {
    // receive size of filename
    let filename_size = stream.read_u64().await? as usize;
    // receive filename
    let mut filename_bytes = vec![0; filename_size];
    stream.read_exact(&mut filename_bytes).await?;
    let filename = String::from_utf8_lossy(&filename_bytes).to_string();
    // receive file size
    let file_size = stream.read_u64().await?;
    Ok((filename, file_size))
}

// returns Ok(true) if we need to perform the transfer
async fn check_for_file(
    filename: &Path,
    size: u64,
    stream: &mut TcpStream,
) -> Result<bool, Box<dyn Error>> {
    // check if file by this name and size exists
    if filename.is_file() {
        // check size
        let metadata = fs::metadata(filename)?;
        let local_size = metadata.len();
        if size == local_size {
            stream.write_u64(1).await?;
            let mut hashes_match = true;
            let local_hash = utils::hash_file(filename)?;
            let mut peer_hash = vec![0; 32];
            stream.read_exact(&mut peer_hash).await?;
            for i in 0..local_hash.len() {
                if local_hash[i] != peer_hash[i] {
                    hashes_match = false;
                }
            }
            stream.write_u64(if hashes_match { 1 } else { 0 }).await?;
            Ok(!hashes_match)
        } else {
            stream.write_u64(0).await?;
            Ok(true)
        }
    } else {
        stream.write_u64(0).await?;
        Ok(true)
    }
}
