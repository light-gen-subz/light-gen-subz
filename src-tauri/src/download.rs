use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::cancel;
use crate::errors::coded;

/// A file with a known size and SHA-256: anything else is refused.
pub struct Expected<'a> {
    pub url: &'a str,
    pub size: u64,
    pub sha256: &'a str,
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            // A stalled connection fails instead of freezing the run forever.
            .read_timeout(Duration::from_secs(60))
            .user_agent(concat!("light-gen-subz/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("building the download client")
    })
}

/// Two downloads must never write the same `.part` file.
fn lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// The hasher after reading all of `path`, to carry on with the bytes that follow.
fn hasher_after(path: &Path) -> Result<Sha256> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).context("opening file to hash")?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).context("reading file to hash")?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher)
}

fn hex(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn sha256_of(path: &Path) -> Result<String> {
    Ok(hex(hasher_after(path)?))
}

/// `true` when `dest` is already there with the expected size. A file of another size
/// (interrupted write, disk full) is deleted so it gets downloaded again.
pub fn is_present(dest: &Path, expected_size: u64) -> bool {
    match std::fs::metadata(dest) {
        Ok(meta) if meta.len() == expected_size => true,
        Ok(_) => {
            let _ = std::fs::remove_file(dest);
            false
        }
        Err(_) => false,
    }
}

/// Downloads `expected.url` to `dest` unless it is already there, resuming an
/// interrupted `.part` file, and checks the SHA-256 before the file becomes visible.
pub async fn ensure_file(
    dest: &Path,
    expected: &Expected<'_>,
    mut on_progress: impl FnMut(f32),
) -> Result<()> {
    let _guard = lock().lock().await;
    if is_present(dest, expected.size) {
        return Ok(());
    }

    let part = part_path(dest);
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > expected.size {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }

    // Hashed while the bytes arrive: the file is not read a second time afterwards.
    let mut streamed: Option<Sha256> = None;
    if have < expected.size {
        let mut request = client().get(expected.url);
        if have > 0 {
            request = request.header("Range", format!("bytes={have}-"));
        }
        let response = request
            .send()
            .await
            .map_err(|e| e.without_url())
            .with_context(|| format!("request to {} failed", expected.url))?
            .error_for_status()
            .map_err(|e| e.without_url())
            .with_context(|| format!("HTTP error response for {}", expected.url))?;

        // 200 means the server ignored the range: start over.
        let resumed = have > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        let mut file = if resumed {
            tokio::fs::OpenOptions::new().append(true).open(&part).await
        } else {
            have = 0;
            tokio::fs::File::create(&part).await
        }
        .context("opening temporary download file")?;

        let mut hasher = if resumed {
            let existing = part.clone();
            tokio::task::spawn_blocking(move || hasher_after(&existing))
                .await
                .context("hashing task")??
        } else {
            Sha256::new()
        };
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            cancel::check()?;
            let chunk = chunk
                .map_err(|e| e.without_url())
                .context("error while streaming the download")?;
            file.write_all(&chunk)
                .await
                .context("writing downloaded file")?;
            hasher.update(&chunk);
            have += chunk.len() as u64;
            on_progress((have as f32 / expected.size as f32).min(1.0));
        }
        file.flush().await.context("flushing downloaded file")?;
        streamed = Some(hasher);
    }

    let actual = match streamed {
        Some(hasher) => hex(hasher),
        None => {
            let part_for_check = part.clone();
            tokio::task::spawn_blocking(move || sha256_of(&part_for_check))
                .await
                .context("hashing task")??
        }
    };
    if actual != expected.sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(coded(
            "download_corrupt",
            format!(
                "downloaded file does not match its expected checksum ({})",
                expected.url
            ),
        ));
    }

    tokio::fs::rename(&part, dest)
        .await
        .context("moving the downloaded file into place")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_file_sits_next_to_the_destination() {
        assert_eq!(
            part_path(Path::new("/m/ggml-small.bin")),
            PathBuf::from("/m/ggml-small.bin.part")
        );
    }

    #[test]
    fn a_file_of_the_expected_size_is_present() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("m.bin");
        std::fs::write(&file, b"12345").unwrap();

        assert!(is_present(&file, 5));
        assert!(file.exists());
    }

    #[test]
    fn a_file_of_another_size_is_removed_to_be_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("m.bin");
        std::fs::write(&file, b"123").unwrap();

        assert!(!is_present(&file, 5));
        assert!(!file.exists());
    }

    #[test]
    fn a_missing_file_is_absent() {
        assert!(!is_present(Path::new("/nowhere/m.bin"), 5));
    }

    #[test]
    fn the_hash_matches_the_known_digest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a");
        std::fs::write(&file, b"abc").unwrap();

        assert_eq!(
            sha256_of(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn a_download_matching_its_checksum_is_installed() {
        let mock = crate::http::testserver::serve(vec![(200, "abc".into())]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");

        runtime()
            .block_on(ensure_file(
                &dest,
                &Expected {
                    url: &mock.url,
                    size: 3,
                    sha256: ABC_SHA,
                },
                |_| {},
            ))
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn a_download_that_does_not_match_is_refused_and_cleaned_up() {
        let mock = crate::http::testserver::serve(vec![(200, "abd".into())]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");

        let err = runtime()
            .block_on(ensure_file(
                &dest,
                &Expected {
                    url: &mock.url,
                    size: 3,
                    sha256: ABC_SHA,
                },
                |_| {},
            ))
            .unwrap_err();

        assert!(err.to_string().contains("checksum"), "{err}");
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
    }

    #[test]
    fn an_existing_file_is_not_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        std::fs::write(&dest, b"abc").unwrap();

        // No server: any request would fail.
        runtime()
            .block_on(ensure_file(
                &dest,
                &Expected {
                    url: "http://127.0.0.1:1/never",
                    size: 3,
                    sha256: ABC_SHA,
                },
                |_| {},
            ))
            .unwrap();
    }

    #[test]
    fn a_complete_part_file_is_checked_without_a_request() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        std::fs::write(part_path(&dest), b"abc").unwrap();

        runtime()
            .block_on(ensure_file(
                &dest,
                &Expected {
                    url: "http://127.0.0.1:1/never",
                    size: 3,
                    sha256: ABC_SHA,
                },
                |_| {},
            ))
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
    }

    #[test]
    fn an_interrupted_download_resumes_from_its_part_file() {
        let mock = crate::http::testserver::serve(vec![(206, "c".into())]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.bin");
        std::fs::write(part_path(&dest), b"ab").unwrap();

        runtime()
            .block_on(ensure_file(
                &dest,
                &Expected {
                    url: &mock.url,
                    size: 3,
                    sha256: ABC_SHA,
                },
                |_| {},
            ))
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        let sent = mock.requests.lock().unwrap();
        assert!(sent[0]
            .headers
            .to_ascii_lowercase()
            .contains("range: bytes=2-"));
    }
}
