use crate::notify::Event;
use tokio::sync::mpsc;
use tracing::{error, info};

pub trait Notifier {
    fn notify_tx(&self) -> &mpsc::UnboundedSender<Event>;

    fn notify_success(&self, quick: bool, message: &str) {
        info!("Success: {message:?}");

        if let Err(e) = self
            .notify_tx()
            .send(Event::Success(quick, message.to_owned()))
        {
            error!("Could not send success {message:?} to notify: {e:?}");
        }
    }

    fn notify_error(&self, message: &str) {
        error!("Error: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Error(message.to_owned())) {
            error!("Could not send error {message:?} to notify: {e:?}");
        }
    }

    fn notify_emergency(&self, message: &str) {
        error!("Emergency: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Emergency(message.to_owned())) {
            error!("Could not send emergency {message:?} to notify: {e:?}");
        }
    }
}
