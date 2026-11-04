use std::time::Duration;

use crate::client::Client;
use crate::error::Error;
use crate::paths::Paths;

pub mod photos;
pub mod plugin;
pub mod raw;
pub mod status;

pub struct Context {
    pub paths: Paths,
    pub timeout: Duration,
    pub connect_timeout: Duration,
}

impl Context {
    pub async fn connect(&self) -> Result<Client, Error> {
        Client::connect(&self.paths.socket(), self.connect_timeout, self.timeout).await
    }
}
