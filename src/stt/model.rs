use std::{
    fs,
    io::{self, ErrorKind, Read},
    path::Path,
};

use anyhow::{Context, Result, bail};

pub const DEFAULT_MODEL_FILE: &str = "ggml-base.en.bin";
const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";

pub fn validate_model_path(path: &Path) -> Result<()> {
    validate_metadata(path, fs::metadata(path))
}

fn validate_metadata(path: &Path, metadata: io::Result<fs::Metadata>) -> Result<()> {
    let metadata = match metadata {
        Err(err) if err.kind() == ErrorKind::NotFound => {
            bail!("missing model file at {}", path.display());
        }
        result => result
            .with_context(|| format!("failed to read model metadata at {}", path.display()))?,
    };
    if !metadata.is_file() {
        bail!(
            "model path must point to a regular file: {}",
            path.display()
        );
    }
    if metadata.len() == 0 {
        bail!("model file is empty: {}", path.display());
    }
    Ok(())
}

pub fn ensure_model(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        metadata => return validate_metadata(path, metadata),
    }
    if path.file_name().and_then(|name| name.to_str()) != Some(DEFAULT_MODEL_FILE) {
        bail!(
            "missing model file at {}. Only the default model is auto-downloaded.",
            path.display()
        );
    }

    eprintln!("Downloading default Whisper model to {}", path.display());
    let response = ureq::get(DEFAULT_MODEL_URL)
        .call()
        .context("failed to download default Whisper model")?;
    save_download(path, response.into_body().into_reader())
        .with_context(|| format!("failed to save model download to {}", path.display()))
}

fn save_download(path: &Path, mut reader: impl Read) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    // A failed or concurrent download must not expose a partial model or
    // replace a model another process (or the user) already installed.
    let mut out = tempfile::NamedTempFile::new_in(parent)?;
    if io::copy(&mut reader, &mut out)? == 0 {
        bail!("downloaded model is empty");
    }
    out.as_file().sync_all()?;
    match out.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(err) if err.error.kind() == ErrorKind::AlreadyExists => validate_model_path(path),
        Err(err) => Err(err.error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_existing_model_is_rejected_without_download() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEFAULT_MODEL_FILE);
        fs::create_dir(&path).unwrap();
        assert!(
            ensure_model(&path)
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
        fs::remove_dir(&path).unwrap();
        fs::write(&path, []).unwrap();
        assert!(
            ensure_model(&path)
                .unwrap_err()
                .to_string()
                .contains("empty")
        );
    }

    #[test]
    fn successful_download_is_published_without_overwriting_an_existing_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models").join(DEFAULT_MODEL_FILE);
        save_download(&path, &b"first model"[..]).unwrap();
        save_download(&path, &b"second model"[..]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first model");
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn failed_or_empty_download_leaves_no_model_or_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEFAULT_MODEL_FILE);
        assert!(save_download(&path, io::empty()).is_err());

        struct BrokenReader;
        impl Read for BrokenReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("connection lost"))
            }
        }

        let interrupted = (&b"partial model"[..]).chain(BrokenReader);
        assert!(save_download(&path, interrupted).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
