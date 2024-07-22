use crate::internal::{notifier::Notifier, online::Online};
use anyhow::{anyhow, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{future::Future, time::Duration};

pub trait Requester: Notifier + Online {
    fn request<Req, Res>(
        &self,
        url: &str,
        method: &reqwest::Method,
        request: Req,
    ) -> impl Future<Output = Result<Res>> + Send
    where
        Self: Sync,
        Req: Serialize + std::fmt::Debug + Send,
        Res: DeserializeOwned,
    {
        async move {
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
}
