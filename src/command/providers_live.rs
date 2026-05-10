use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub(crate) trait ProviderLivePinger {
    fn ping(&self, endpoint: &str) -> std::result::Result<(), String>;
}

#[cfg(test)]
pub(crate) struct DisabledLivePinger;

#[cfg(test)]
impl ProviderLivePinger for DisabledLivePinger {
    fn ping(&self, _endpoint: &str) -> std::result::Result<(), String> {
        Ok(())
    }
}

pub(crate) struct TcpProviderLivePinger;

impl ProviderLivePinger for TcpProviderLivePinger {
    fn ping(&self, endpoint: &str) -> std::result::Result<(), String> {
        let mut addrs = endpoint
            .to_socket_addrs()
            .map_err(|err| format!("resolve failed: {err}"))?;
        let addr = addrs
            .next()
            .ok_or_else(|| "no socket address".to_string())?;
        TcpStream::connect_timeout(&addr, Duration::from_millis(1_500))
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}
