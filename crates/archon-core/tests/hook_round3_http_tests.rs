use archon_core::hooks::HookHttpTransport as Client;
use archon_core::hooks::{HookConfig, execute_http_hook};
use reqwest::{Certificate, Identity, header::HeaderMap};
use std::process::Stdio;
use std::time::Duration;
#[path = "support/hook_tls.rs"]
mod tls;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::header};

fn config(url: String) -> HookConfig {
    serde_json::from_value(serde_json::json!({
        "type":"http", "command":url, "timeout":2, "on_failure":"allow"
    }))
    .unwrap()
}
#[tokio::test]
async fn transport_preserves_authentication_headers() {
    let server = MockServer::start().await;
    Mock::given(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "outcome":"success", "additional_context":"authenticated"
        })))
        .mount(&server)
        .await;
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer test-token".parse().unwrap());
    let client = Client::builder().default_headers(headers).build().unwrap();
    let result = execute_http_hook(&config(server.uri()), &serde_json::json!({}), &client).await;
    assert_eq!(
        result.additional_context.as_deref(),
        Some("authenticated"),
        "{result:?}"
    );
}
#[tokio::test]
async fn transport_preserves_redirect_policy() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::path("/hook"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", "/end"))
        .mount(&server)
        .await;
    Mock::given(wiremock::matchers::path("/end"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"outcome":"success"})),
        )
        .mount(&server)
        .await;
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let _ = execute_http_hook(
        &config(format!("{}/hook", server.uri())),
        &serde_json::json!({}),
        &client,
    )
    .await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1, "redirect policy discarded");
}
struct TlsServer(std::process::Child, tls::Material);
impl Drop for TlsServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn tls_server(require_client: bool) -> (String, TlsServer) {
    let material = tls::generate();
    let dir = tempfile::tempdir().unwrap();
    let port = dir.path().join("port");
    let script = r#"
import ssl, socket, sys
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(sys.argv[1]+'/server.pem', sys.argv[1]+'/server-key.pem')
if sys.argv[3] == 'true':
    ctx.load_verify_locations(sys.argv[1]+'/ca.pem')
    ctx.verify_mode = ssl.CERT_REQUIRED
s = socket.socket(); s.bind(('127.0.0.1', 0)); s.listen(1)
with open(sys.argv[2], 'w') as f: f.write(str(s.getsockname()[1]))
c, _ = s.accept()
with ctx.wrap_socket(c, server_side=True) as conn:
    conn.recv(65536)
    body = b'{"outcome":"success","additional_context":"private TLS"}'
    conn.sendall(b'HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: '+str(len(body)).encode()+b'\r\n\r\n'+body)
"#;
    let child = archon_shell::spawn::command("python3")
        .arg("-c")
        .arg(script)
        .arg(material.dir.path())
        .arg(&port)
        .arg(require_client.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let guard = TlsServer(child, material);
    let port = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&port)
                && text.parse::<u16>().is_ok()
            {
                break text;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("TLS server startup");
    (format!("https://127.0.0.1:{port}"), guard)
}
async fn private_tls(require_client: bool) {
    let (url, server) = tls_server(require_client).await;
    let mut builder =
        Client::builder().add_root_certificate(Certificate::from_pem(&server.1.ca).unwrap());
    if require_client {
        builder = builder.identity(Identity::from_pem(&server.1.client_identity).unwrap());
    }
    let result = execute_http_hook(
        &config(url),
        &serde_json::json!({}),
        &builder.build().unwrap(),
    )
    .await;
    assert_eq!(
        result.additional_context.as_deref(),
        Some("private TLS"),
        "{result:?}"
    );
}
#[tokio::test]
async fn transport_preserves_private_ca() {
    private_tls(false).await;
}
#[tokio::test]
async fn transport_preserves_client_certificate() {
    private_tls(true).await;
}

#[tokio::test]
async fn tls_server_key_is_generated_at_runtime() {
    assert!(
        !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hook-tls/server-key.pem")
            .exists(),
        "static server private key"
    );
    private_tls(false).await;
}
#[tokio::test]
async fn tls_client_identity_is_generated_at_runtime() {
    assert!(
        !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hook-tls/client-identity.pem")
            .exists(),
        "static client private key"
    );
    private_tls(true).await;
}
#[test]
fn tls_ca_is_generated_at_runtime() {
    assert!(
        !std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hook-tls/ca.pem")
            .exists(),
        "static CA material"
    );
    let first = tls::generate();
    let second = tls::generate();
    assert_ne!(first.ca, second.ca, "CA reused across test runs");
    assert_ne!(
        first.client_identity, second.client_identity,
        "client identity reused"
    );
    let ca = openssl::x509::X509::from_pem(&first.ca).unwrap();
    let server =
        openssl::x509::X509::from_pem(&std::fs::read(first.dir.path().join("server.pem")).unwrap())
            .unwrap();
    assert!(server.verify(&ca.public_key().unwrap()).unwrap());
    let client = openssl::x509::X509::from_pem(&first.client_identity).unwrap();
    assert!(client.verify(&ca.public_key().unwrap()).unwrap());
}
