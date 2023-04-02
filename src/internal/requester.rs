use crate::internal::{notifier::Notifier, online::Online};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

#[async_trait]
pub trait Requester: Notifier + Online {
    async fn request<Req, Res>(
        &self,
        url: &str,
        method: &reqwest::Method,
        request: Req,
    ) -> Result<Res>
    where
        Req: Serialize + std::fmt::Debug + Send + Sync,
        Res: DeserializeOwned + std::fmt::Debug + Send,
    {
        let builder = reqwest::ClientBuilder::new().timeout(Duration::from_secs(10));
        let client = builder.build()?;

        let res = match client
            .request(method.clone(), url)
            .json(&request)
            .send()
            .await
        {
            Ok(res) => res,
            Err(e) => {
                self.observed_error(&e);
                return Err(anyhow!(e));
            }
        };

        self.observed_online();

        if !res.status().is_success() {
            return Err(anyhow!(
                "Error {method:?}ing {url:?} with {request:?}: got status code {}",
                res.status()
            ));
        }

        match res.json().await {
            Ok(j) => Ok(j),
            Err(e) => Err(anyhow!(e)),
        }
    }
}
