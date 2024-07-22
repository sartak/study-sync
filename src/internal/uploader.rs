use crate::internal::{notifier::Notifier, online::Online};
use anyhow::{anyhow, Result};
use reqwest::Body;
use sha1::{Digest, Sha1};
use std::{
    future::Future,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::fs::File;
use tokio_util::codec::{BytesCodec, FramedRead};
use tracing::info;

pub trait Uploader: Notifier + Send + Online {
    fn get_digest_cache(&self) -> &Option<(PathBuf, String)>;
    fn set_digest_cache(&mut self, cache: Option<(PathBuf, String)>);

    fn digest_for_path(&mut self, path: &Path) -> impl Future<Output = Option<String>> + Send {
        async move {
            if let Some((p, d)) = self.get_digest_cache() {
                if p == path {
                    return Some(d.clone());
                }
            }

            let res = {
                let path = path.to_owned();
                tokio::task::spawn_blocking(move || -> Result<String> {
                    let mut file = std::fs::File::open(path)?;
                    let mut hasher = Sha1::new();
                    std::io::copy(&mut file, &mut hasher)?;
                    Ok(hex::encode(hasher.finalize()))
                })
            }
            .await;

            match res {
                Ok(Ok(digest)) => {
                    self.set_digest_cache(Some((path.to_path_buf(), digest.clone())));
                    Some(digest)
                }
                Ok(Err(e)) => {
                    self.notify_error(&format!("Could not calculate digest of {path:?}: {e:?}"));
                    None
                }
                Err(e) => {
                    self.notify_error(&format!(
                        "Could not join future for calculating digest of {path:?}: {e:?}"
                    ));
                    None
                }
            }
        }
    }

    fn upload_path_to_directory(
        &mut self,
        base_url: &str,
        path: &Path,
        directory: &str,
        content_type: Option<&str>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            let mut url = format!("{base_url}/{directory}");

            let basename = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("");

            if let Some(digest) = self.digest_for_path(path).await {
                let param = format!("?digest={digest}");
                url.push_str(&param);
            }

            let file = File::open(path).await?;
            let stream = FramedRead::new(file, BytesCodec::new());
            let body = Body::wrap_stream(stream);

            let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(30));
            let client = builder.build()?;

            let mut req = client
                .post(&url)
                .header("X-Study-Basename", basename)
                .body(body);

            if let Some(content_type) = content_type {
                req = req.header(reqwest::header::CONTENT_TYPE, content_type);
            }

            let res = match req.send().await {
                Ok(res) => res,
                Err(e) => {
                    self.observed_error(&e);
                    return Err(anyhow!(e));
                }
            };

            self.observed_online();

            if !res.status().is_success() {
                return Err(anyhow!(
                    "Failed to upload {path:?} using {url:?}: got status code {}, body {:?}",
                    res.status(),
                    res.text().await
                ));
            }

            let message = res.text().await?;
            info!("Successfully uploaded {path:?} to {url:?}: {message}");

            Ok(())
        }
    }
}
