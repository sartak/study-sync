use crate::notify::Event;
use log::{error, info};
use tokio::sync::mpsc;

pub trait Notifier {
    fn notify_target(&self) -> &str;

    fn notify_tx(&self) -> &mpsc::UnboundedSender<Event>;

    fn notify_success(&self, quick: bool, message: &str) {
        info!(target: self.notify_target(), "Success: {message:?}");

        if let Err(e) = self
            .notify_tx()
            .send(Event::Success(quick, message.to_owned()))
        {
            error!(target: self.notify_target(), "Could not send success {message:?} to notify: {e:?}");
        }
    }

    fn notify_error(&self, message: &str) {
        error!(target: self.notify_target(), "Error: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Error(message.to_owned())) {
            error!(target: self.notify_target(), "Could not send error {message:?} to notify: {e:?}");
        }
    }

    fn notify_emergency(&self, message: &str) {
        error!(target: self.notify_target(), "Emergency: {message:?}");

        if let Err(e) = self.notify_tx().send(Event::Emergency(message.to_owned())) {
            error!(target: self.notify_target(), "Could not send emergency {message:?} to notify: {e:?}");
        }
    }
}
