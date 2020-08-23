use log::{info};
use std::time::{SystemTime, Duration};

pub struct Prof(SystemTime);

impl Prof {
    pub fn new() -> Prof {
        Prof(SystemTime::now())
    }

    pub fn log(&mut self, name: &str) {
        let elapsed = self.0.elapsed().unwrap_or_else(|_| Duration::from_millis(0));
        info!("{} took {}ms", name, elapsed.as_millis());
        *self = Self::new();
    }
}