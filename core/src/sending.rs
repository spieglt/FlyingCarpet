use crate::{chunksize, utils, FCError, UI};
use std::{
    fs::{metadata, File},
    io::Read,
    path::Path,
    time::Instant,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

// v10+: file contents are protected by the Noise transport (see core/src/noise.rs), which
// wraps the whole connection, so chunks are sent as raw bytes here — no separate
// application-level encryption.
// `relative_name` is the path the peer stores the file under, relative to the folder they
// chose, always with "/" separators. It is resolved when the user makes their selection
// (utils::expand_selection), not here.
pub async fn send_file<S: AsyncRead + AsyncWrite + Unpin, T: UI>(
    file: &Path,
    relative_name: &str,
    stream: &mut S,
    ui: &T,
) -> Result<(), FCError> {
    let start = Instant::now();
    let mut handle = File::open(file)?;
    let metadata = metadata(file)?;
    let size = metadata.len();
    let mut bytes_left = size;
    ui.output(&format!("File size: {}", utils::make_size_readable(size)));

    // send file details
    send_file_details(relative_name, size, stream).await?;

    // check to see if receiving end already has the file
    let need_transfer = check_for_file(&file, stream).await?;
    if !need_transfer {
        ui.output("Recipient already has this file, skipping.");
        return Ok(());
    }

    // show progress bar
    ui.show_progress_bar();

    let mut buffer = vec![0u8; chunksize()];

    // TEMPORARY diagnostic (2026-07-25): split the loop's wall time into "waiting on the
    // disk" and "waiting on the socket" so a slow transfer says which one it is. write_all
    // returns once the kernel takes the bytes, so time piling up there means the send
    // buffer is full and we are really waiting on the peer to consume.
    let mut read_time = std::time::Duration::ZERO;
    let mut write_time = std::time::Duration::ZERO;
    let mut chunks = 0u64;

    // Report every few seconds rather than only at the end, so a slow transfer can be
    // diagnosed from the first minute instead of requiring the whole file. The per-interval
    // numbers are the useful ones -- a cumulative average flattens out exactly the
    // variation worth seeing.
    const REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
    let mut last_report = Instant::now();
    let mut interval_read = std::time::Duration::ZERO;
    let mut interval_write = std::time::Duration::ZERO;
    let mut interval_chunks = 0u64;
    let mut interval_bytes = 0u64;

    while bytes_left > 0 {
        tokio::task::yield_now().await;
        let read_start = Instant::now();
        let read_result = handle.read(&mut buffer);
        let this_read = read_start.elapsed();
        read_time += this_read;
        match read_result {
            Ok(bytes_read) if bytes_read == 0 => {
                // EOF, shouldn't hit this due to while loop condition
                ui.output("Hit EOF");
                break;
            }
            Ok(bytes_read) => {
                bytes_left -= bytes_read as u64;
                let write_start = Instant::now();
                send_chunk(&buffer[..bytes_read], stream).await?;
                let this_write = write_start.elapsed();
                write_time += this_write;
                chunks += 1;

                interval_read += this_read;
                interval_write += this_write;
                interval_chunks += 1;
                interval_bytes += bytes_read as u64;
                if last_report.elapsed() >= REPORT_EVERY {
                    let secs = last_report.elapsed().as_secs_f64();
                    let per = interval_chunks.max(1) as f64;
                    ui.output(&format!(
                        "Diag: {:.1}mbps over last {:.0}s — {} chunks: disk {:.1}ms/chunk, socket {:.1}ms/chunk",
                        8.0 * (interval_bytes as f64 / 1_000_000.0) / secs,
                        secs,
                        interval_chunks,
                        interval_read.as_secs_f64() * 1000.0 / per,
                        interval_write.as_secs_f64() * 1000.0 / per,
                    ));
                    interval_read = std::time::Duration::ZERO;
                    interval_write = std::time::Duration::ZERO;
                    interval_chunks = 0;
                    interval_bytes = 0;
                    last_report = Instant::now();
                }

                let percent_done = ((size - bytes_left) as f64 / size as f64) * 100.;
                ui.update_progress_bar(percent_done as u8);
            }
            Err(e) => Err(e)?,
        }
    }

    // send chunkSize of 0
    stream.write_u64(0).await?;

    // stats
    ui.update_progress_bar(100);
    let finish = Instant::now();
    let elapsed = (finish - start).as_secs_f64();
    ui.output(&format!("Sending took {}", utils::format_time(elapsed)));

    let megabits = 8.0 * (size as f64 / 1_000_000.0);
    let mbps = megabits / elapsed;
    ui.output(&format!("Speed: {:.2}mbps", mbps));

    // TEMPORARY diagnostic (2026-07-25): remove once the slow-transfer question is settled.
    // Disk-dominated means the read pattern is the problem; socket-dominated means we are
    // blocked on the peer; neither dominating means the per-chunk overhead is elsewhere.
    ui.output(&format!(
        "Diag: {} chunks of {}KB — disk read {:.1}s ({:.1}ms/chunk), socket write {:.1}s ({:.1}ms/chunk), loop total {:.1}s",
        chunks,
        chunksize() / 1024,
        read_time.as_secs_f64(),
        read_time.as_secs_f64() * 1000.0 / chunks.max(1) as f64,
        write_time.as_secs_f64(),
        write_time.as_secs_f64() * 1000.0 / chunks.max(1) as f64,
        elapsed,
    ));

    // listen for receiving end to tell us they have everything
    stream.read_u64().await?;

    // send double confirmation
    // std::thread::sleep(std::time::Duration::from_secs(5));
    stream.write_u64(1).await?;

    Ok(())
}

async fn send_chunk<S: AsyncWrite + Unpin>(chunk: &[u8], stream: &mut S) -> Result<(), FCError> {
    // length-prefixed raw bytes; confidentiality/integrity come from the Noise transport
    stream.write_u64(chunk.len() as u64).await?;
    stream.write_all(chunk).await?;
    Ok(())
}

async fn send_file_details<S: AsyncWrite + Unpin>(
    filename: &str,
    size: u64,
    stream: &mut S,
) -> std::io::Result<()> {
    // send size of filename
    stream.write_u64(filename.len() as u64).await?;
    // send filename
    stream.write_all(filename.as_bytes()).await?;
    // send file size
    stream.write_u64(size).await?;
    Ok(())
}

// returns Ok(true) if we need to perform the transfer
async fn check_for_file<S: AsyncRead + AsyncWrite + Unpin>(
    filename: &Path,
    stream: &mut S,
) -> Result<bool, FCError> {
    let has_file = stream.read_u64().await?;
    if has_file == 1 {
        let hash = utils::hash_file(filename)?;
        // write_all, not write: a short write would send a truncated hash and leave the
        // rest of it to be read as the next protocol field
        stream.write_all(&hash).await?;
        let hashes_match = stream.read_u64().await?;
        Ok(hashes_match != 1) // if hashes match, return false because we don't need transfer
    } else {
        Ok(true)
    }
}

/*
mod tests {
    use tokio::io::AsyncReadExt;

    // nc -l 4387
    // test that timeout closes tcp connection early
    #[tokio::test]
    async fn timeout() {
        let addr = "127.0.0.1:4387".parse::<std::net::SocketAddr>().unwrap();
        println!("waiting...");
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let data = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read_u64()).await;
        println!("{:?}", data);
        println!("timed out after 5 seconds");
    }
}
*/
