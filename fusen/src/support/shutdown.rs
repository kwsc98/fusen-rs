use tokio::sync::broadcast;

#[derive(Debug)]
pub struct Shutdown {
    shutdown: bool,
    notify: broadcast::Receiver<()>,
}

impl Shutdown {
    pub fn new(notify: broadcast::Receiver<()>) -> Self {
        Shutdown {
            shutdown: false,
            notify,
        }
    }
}

impl Shutdown {
    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    pub fn _shutdown(&mut self) {
        self.shutdown = true;
    }

    pub async fn recv(&mut self) {
        if self.is_shutdown() {
            return;
        }
        let _ = self.notify.recv().await;
        self.shutdown = true;
    }
}
