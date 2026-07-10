use crate::{error::fc_error, utils, FCError, UI};
use core::time;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::{sleep, timeout},
};

// Sanity bounds on peer-supplied header values, checked before they're used to size an
// allocation (see also MAX_FILE_COUNT in lib.rs). Chosen so every legitimate transfer
// passes — Apple and Android senders use 5MB chunks, ours are CHUNKSIZE (1MB) — while
// absurd values, which are memory-exhaustion levers, are rejected as corrupt/hostile.
const MAX_FILENAME_BYTES: u64 = 8192;
const MAX_CHUNK_BYTES: u64 = 5_000_000;

// v10+: file contents are protected by the Noise transport (see core/src/noise.rs), which
// wraps the whole connection, so chunks arrive as raw bytes here — no application-level
// decryption.
pub async fn receive_file<S: AsyncRead + AsyncWrite + Unpin, T: UI>(
    folder: &Path,
    stream: &mut S,
    ui: &T,
    last_file: bool,
) -> Result<(), FCError> {
    let folder = folder.to_owned();
    let start = Instant::now();

    // check destination folder
    fs::read_dir(&folder)?;

    // receive file details
    let (filename, file_size) = receive_file_details(stream).await?;
    ui.output(&format!("Filename: {}", filename));
    ui.output(&format!(
        "File size: {}",
        utils::make_size_readable(file_size)
    ));
    let mut bytes_left = file_size;

    // see if we already have the file being sent
    let relative_path = sanitize_relative_filename(&filename)?;
    let mut full_path = folder.clone();
    full_path.push(&relative_path);
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
        let chunk = receive_chunk(stream).await?;
        if chunk.len() == 0 {
            break;
        }
        // saturating: a peer that sends more data than the advertised file size must
        // not underflow (the loop is bounded by the end-of-file sentinel, not by this)
        bytes_left = bytes_left.saturating_sub(chunk.len() as u64);
        out_file.write_all(&chunk)?;
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

async fn receive_chunk<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>, FCError> {
    // receive chunk size. 0 is the legitimate end-of-file sentinel; a larger-than-possible
    // value means a corrupt or hostile stream, and must be rejected before we allocate a
    // receive buffer of that size.
    let chunk_size = stream.read_u64().await?;
    if chunk_size == 0 {
        return Ok(vec![]);
    }
    if chunk_size > MAX_CHUNK_BYTES {
        fc_error(&format!(
            "Chunk size {} from peer is out of range",
            chunk_size
        ))?;
    }
    // receive chunk (raw bytes; the Noise transport already authenticated and decrypted it)
    let mut chunk = vec![0u8; chunk_size as usize];
    stream.read_exact(&mut chunk).await?;
    Ok(chunk)
}

fn sanitize_relative_filename(filename: &str) -> Result<PathBuf, FCError> {
    let mut sanitized = PathBuf::new();
    for component in Path::new(filename).components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => (),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                fc_error(&format!("Received invalid filename path: {}", filename))?
            }
        }
    }
    if sanitized.as_os_str().is_empty() {
        fc_error("Received empty filename")?;
    }
    Ok(sanitized)
}

async fn receive_file_details<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<(String, u64), FCError> {
    // receive size of filename. real paths fit comfortably under the bound; an
    // unbounded value is a memory-exhaustion lever, so reject before allocating.
    let filename_size = stream.read_u64().await?;
    if filename_size > MAX_FILENAME_BYTES {
        fc_error(&format!(
            "Filename length {} from peer is out of range",
            filename_size
        ))?;
    }
    // receive filename
    let mut filename_bytes = vec![0; filename_size as usize];
    stream.read_exact(&mut filename_bytes).await?;
    let filename = String::from_utf8(filename_bytes)?;
    // receive file size
    let file_size = stream.read_u64().await?;
    Ok((filename, file_size))
}

// returns Ok(true) if we need to perform the transfer
async fn check_for_file<S: AsyncRead + AsyncWrite + Unpin>(
    filename: &Path,
    size: u64,
    stream: &mut S,
) -> Result<bool, FCError> {
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
            // TODO: ugly hack to get around lifetime issue? sending end didn't receive this last reply when calculating hash of large file.
            sleep(time::Duration::from_secs(1)).await;
            Ok(true)
        }
    } else {
        stream.write_u64(0).await?;
        // TODO: ugly hack to get around lifetime issue? sending end didn't receive this last reply when calculating hash of large file.
        sleep(time::Duration::from_secs(1)).await;
        Ok(true)
    }
}
