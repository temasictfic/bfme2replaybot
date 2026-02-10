use std::io::Read;

const MAX_REPLAYS_PER_ARCHIVE: usize = 100;
const MAX_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 500 * 1024 * 1024; // 500MB total
const MAX_ARCHIVE_EXTRACTED_FILES: usize = 200;
const MAX_SINGLE_REPLAY_BYTES: u64 = 5 * 1024 * 1024; // 5MB

/// Extract .BfME2Replay files from a ZIP archive (in-memory).
/// Returns (replays, total_count) — only up to MAX_REPLAYS_PER_ARCHIVE are extracted,
/// but total_count reflects how many were found.
pub fn extract_replays_from_zip(data: &[u8]) -> (Vec<(String, Vec<u8>)>, usize) {
    let cursor = std::io::Cursor::new(data);
    let mut archive = match zip::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Failed to open ZIP archive: {}", e);
            return (Vec::new(), 0);
        }
    };

    let mut replays = Vec::new();
    let mut total = 0usize;
    let mut extracted_bytes: u64 = 0;

    for i in 0..archive.len() {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("Failed to read ZIP entry {}: {}", i, e);
                continue;
            }
        };

        let name = file.name().to_string();
        if !name.to_lowercase().ends_with(".bfme2replay") || file.is_dir() {
            continue;
        }

        total += 1;

        // Count but don't extract beyond the cap
        if replays.len() >= MAX_REPLAYS_PER_ARCHIVE {
            continue;
        }

        // Skip files larger than 5MB
        if file.size() > MAX_SINGLE_REPLAY_BYTES {
            tracing::warn!(
                "Skipping oversized replay in ZIP: {} ({} bytes)",
                name,
                file.size()
            );
            continue;
        }

        // Check total uncompressed bytes before allocating
        extracted_bytes += file.size();
        if extracted_bytes > MAX_ARCHIVE_UNCOMPRESSED_BYTES {
            tracing::warn!(
                "ZIP extraction byte limit exceeded ({} bytes), stopping",
                extracted_bytes
            );
            break;
        }

        // Use Read::take to cap actual bytes read
        let mut buf = Vec::with_capacity(file.size() as usize);
        if let Err(e) = file
            .by_ref()
            .take(MAX_SINGLE_REPLAY_BYTES)
            .read_to_end(&mut buf)
        {
            tracing::warn!("Failed to extract {}: {}", name, e);
            continue;
        }

        // Use just the filename, not the full path inside the archive
        let short_name = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();

        replays.push((short_name, buf));
    }

    (replays, total)
}

/// Extract .BfME2Replay files from a RAR archive (via temp directory).
/// Returns (replays, total_count) — only up to MAX_REPLAYS_PER_ARCHIVE bytes are read,
/// but total_count reflects how many replay files were found on disk.
pub fn extract_replays_from_rar(data: &[u8]) -> (Vec<(String, Vec<u8>)>, usize) {
    let tmp_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to create temp dir: {}", e);
            return (Vec::new(), 0);
        }
    };

    // Write RAR data to a temp file (unrar needs a filesystem path)
    let rar_path = tmp_dir.path().join("archive.rar");
    if let Err(e) = std::fs::write(&rar_path, data) {
        tracing::error!("Failed to write temp RAR file: {}", e);
        return (Vec::new(), 0);
    }

    let extract_dir = tmp_dir.path().join("extracted");
    if let Err(e) = std::fs::create_dir_all(&extract_dir) {
        tracing::error!("Failed to create extract dir: {}", e);
        return (Vec::new(), 0);
    }

    // Extract using unrar
    let mut archive =
        match unrar::Archive::new::<str>(&rar_path.to_string_lossy()).open_for_processing() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!("Failed to open RAR archive: {}", e);
                return (Vec::new(), 0);
            }
        };

    // Extract all files (unrar API requires sequential processing)
    let mut extracted_bytes: u64 = 0;
    let mut extracted_files: usize = 0;
    loop {
        let header = match archive.read_header() {
            Ok(Some(header)) => header,
            Ok(None) => break,
            Err(e) => {
                tracing::error!("Failed to read RAR header: {}", e);
                break;
            }
        };

        let is_file = header.entry().is_file();
        let unpacked = header.entry().unpacked_size;

        if is_file {
            extracted_files += 1;
            extracted_bytes += unpacked;
            if extracted_bytes > MAX_ARCHIVE_UNCOMPRESSED_BYTES
                || extracted_files > MAX_ARCHIVE_EXTRACTED_FILES
            {
                tracing::warn!(
                    "RAR extraction limits exceeded ({} bytes, {} files), stopping",
                    extracted_bytes,
                    extracted_files
                );
                let _ = header.skip();
                break;
            }
            archive = match header.extract_with_base(&extract_dir) {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("Failed to extract RAR entry: {}", e);
                    break;
                }
            };
        } else {
            archive = match header.skip() {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!("Failed to skip RAR entry: {}", e);
                    break;
                }
            };
        }
    }

    // Collect extracted .BfME2Replay files (reads bytes only up to cap)
    let mut replays = Vec::new();
    let mut total = 0usize;
    collect_replay_files(&extract_dir, &mut replays, &mut total);

    (replays, total)
    // tmp_dir is dropped here, cleaning up all temp files
}

/// Recursively collect .BfME2Replay files from a directory.
/// Only reads file bytes for the first MAX_REPLAYS_PER_ARCHIVE files; counts the rest.
fn collect_replay_files(
    dir: &std::path::Path,
    replays: &mut Vec<(String, Vec<u8>)>,
    total: &mut usize,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_replay_files(&path, replays, total);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && name.to_lowercase().ends_with(".bfme2replay")
        {
            *total += 1;

            // Count but don't read bytes beyond the cap
            if replays.len() >= MAX_REPLAYS_PER_ARCHIVE {
                continue;
            }

            // Skip files larger than 5MB
            if let Ok(meta) = path.metadata()
                && meta.len() > MAX_SINGLE_REPLAY_BYTES
            {
                tracing::warn!("Skipping oversized replay: {} ({} bytes)", name, meta.len());
                continue;
            }

            match std::fs::read(&path) {
                Ok(bytes) => replays.push((name.to_string(), bytes)),
                Err(e) => tracing::warn!("Failed to read {}: {}", name, e),
            }
        }
    }
}
