use crate::{internal::notifier::Notifier, orchestrator};
use log::info;
use tokio::sync::mpsc;

pub trait Online: Notifier {
    fn orchestrator_tx(&self) -> &mpsc::UnboundedSender<orchestrator::Event>;
    fn is_online(&self) -> bool;

    fn observed_is_online(&self, is_online: bool) {
        if self.is_online() == is_online {
            return;
        }

        info!(
            "Observed online status changed: {:?} -> {is_online:?}",
            self.is_online()
        );

        if let Err(e) = self
            .orchestrator_tx()
            .send(orchestrator::Event::IsOnline(is_online))
        {
            self.notify_error(&format!("Could not send to orchestrator: {e:?}"));
        }
    }

    fn observed_online(&self) {
        self.observed_is_online(true);
    }

    fn observed_offline(&self) {
        self.observed_is_online(false);
    }

    fn observed_error(&self, error: &reqwest::Error) {
        if error.is_timeout() || format!("{error:?}").contains("ConnectError") {
            self.observed_offline();
        }
    }
}
