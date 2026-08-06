use super::*;

/// Ask the OS for a free port and hand it back. Racy in principle; in practice
/// the alternative is a hardcoded port that collides with whatever the
/// developer already has running.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe listener");
    let port = listener.local_addr().expect("probe addr").port();
    drop(listener);
    port
}

async fn health_ok(url: &str) -> bool {
    reqwest::Client::new()
        .get(format!("{url}/health"))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn options(port: u16) -> AttachOptions {
    AttachOptions {
        port,
        working_dir: std::env::temp_dir(),
        memory: None,
    }
}

// The attached server lives in a process-global slot, so these two must not
// overlap.
#[serial_test::serial(attached_web)]
#[tokio::test]
async fn attached_server_answers_health_and_stops_with_the_session() {
    let port = free_port();
    let url = start(options(port)).expect("start attached web server");
    assert_eq!(url, format!("http://127.0.0.1:{port}"));

    // The listener binds inside the spawned task, so poll rather than assume
    // it is up the instant `start` returns.
    let mut healthy = false;
    for _ in 0..50 {
        if health_ok(&url).await {
            healthy = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(healthy, "attached server never answered /health at {url}");
    assert_eq!(running_url().as_deref(), Some(url.as_str()));

    // Session teardown.
    shutdown().await;

    assert!(running_url().is_none());
    assert!(
        !health_ok(&url).await,
        "server outlived the session that spawned it"
    );
}

#[serial_test::serial(attached_web)]
#[tokio::test]
async fn starting_twice_reports_the_running_server_instead_of_binding_again() {
    let port = free_port();
    let url = start(options(port)).expect("start attached web server");
    let error = start(options(free_port())).expect_err("second start must be refused");
    assert!(error.to_string().contains(&url), "{error:#}");
    shutdown().await;
}
