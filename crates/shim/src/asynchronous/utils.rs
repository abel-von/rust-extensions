use crate::error::Error;
use crate::error::Result;
use std::fmt::format;
use std::path::Path;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::spawn_blocking;

pub(crate) async fn asyncify<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    spawn_blocking(f)
        .await
        .map_err(other_error!(e, "failed to spawn blocking task"))?
}

pub async fn read_file_to_str(path: impl AsRef<Path>) -> Result<String> {
    let mut file = tokio::fs::File::open(&path).await.map_err(io_error!(
        e,
        "failed to open file {}",
        path.as_ref().display()
    ))?;

    let mut content = String::new();
    file.read_to_string(&mut content).await.map_err(io_error!(
        e,
        "failed to read {}",
        path.as_ref().display()
    ))?;
    Ok(content)
}

pub async fn write_str_to_file(filename: impl AsRef<Path>, s: impl AsRef<str>) -> Result<()> {
    let file = filename.as_ref().file_name().ok_or_else(|| {
        Error::InvalidArgument(format!("pid path illegal {}", filename.as_ref().display()))
    })?;
    let tmp_path = filename
        .as_ref()
        .parent()
        .map(|x| x.join(format!(".{}", file.to_str().unwrap_or(""))))
        .ok_or_else(|| Error::InvalidArgument(String::from("failed to create tmp path")))?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .await
        .map_err(io_error!(e, "open {}", tmp_path.display()))?;
    f.write_all(s.as_ref().as_bytes()).await.map_err(io_error!(
        e,
        "write tmp file {}",
        tmp_path.display()
    ))?;
    tokio::fs::rename(&tmp_path, &filename)
        .await
        .map_err(io_error!(
            e,
            "rename tmp file to {}",
            filename.as_ref().display()
        ))?;
    Ok(())
}
