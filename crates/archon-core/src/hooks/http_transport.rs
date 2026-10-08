//! Hook transports preserve connection settings while exposing no runtime clocks.
use reqwest::{Certificate, Client, ClientBuilder, Identity, Proxy, header::HeaderMap};

/// A reusable hook client with no total or read timeout.
/// Construct this instead of an opaque reqwest Client: reqwest cannot remove
/// or inspect those clocks on an already-built client.
#[derive(Debug, Clone)]
pub struct HookHttpTransport(Client);

impl Default for HookHttpTransport {
    fn default() -> Self {
        Self(Client::new())
    }
}
impl HookHttpTransport {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn builder() -> HookHttpTransportBuilder {
        HookHttpTransportBuilder(Client::builder())
    }
}

/// Only connection configuration is exposed. There is deliberately no escape
/// hatch to a raw builder that could install total/read clocks.
pub struct HookHttpTransportBuilder(ClientBuilder);
impl HookHttpTransportBuilder {
    pub fn default_headers(mut self, headers: HeaderMap) -> Self {
        self.0 = self.0.default_headers(headers);
        self
    }
    pub fn add_root_certificate(mut self, certificate: Certificate) -> Self {
        self.0 = self.0.add_root_certificate(certificate);
        self
    }
    pub fn identity(mut self, identity: Identity) -> Self {
        self.0 = self.0.identity(identity);
        self
    }
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.0 = self.0.proxy(proxy);
        self
    }
    pub fn no_proxy(mut self) -> Self {
        self.0 = self.0.no_proxy();
        self
    }
    pub fn redirect(mut self, policy: reqwest::redirect::Policy) -> Self {
        self.0 = self.0.redirect(policy);
        self
    }
    pub fn resolve(mut self, domain: &str, address: std::net::SocketAddr) -> Self {
        self.0 = self.0.resolve(domain, address);
        self
    }
    pub fn build(self) -> Result<HookHttpTransport, reqwest::Error> {
        self.0.build().map(HookHttpTransport)
    }
}

impl HookHttpTransport {
    /// The configured client, with no total or read clock installed.
    pub(super) fn client(&self) -> &Client {
        &self.0
    }
}
