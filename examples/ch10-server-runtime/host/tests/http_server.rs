use std::path::{Path, PathBuf};
use std::sync::Arc;

use ch10_host::http::{MAX_REQUEST_BODY_BYTES, router};
use ch10_host::{RequestLimits, ServerRuntime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the wasm32-wasip2 guest artifact; see the chapter README"]
async fn component_is_invoked_over_http() {
    let path =
        std::env::var("CH10_COMPONENT").expect("CH10_COMPONENT must name the guest artifact");
    let runtime = Arc::new(
        ServerRuntime::load(
            workspace_relative(Path::new(&path)),
            RequestLimits::default(),
        )
        .expect("component should compile once"),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral HTTP listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have a local address");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(runtime))
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("HTTP server should run");
    });

    let success = send_request(address, "POST", "/uppercase", b"http server").await;
    assert!(success.starts_with("HTTP/1.1 200 OK\r\n"));
    assert_eq!(response_body(&success), "HTTP SERVER");

    let rejection = send_request(address, "POST", "/reject", b"private guest detail").await;
    assert!(rejection.starts_with("HTTP/1.1 400 Bad Request\r\n"));
    assert_eq!(response_body(&rejection), "request rejected");

    let oversized_body = vec![b'x'; MAX_REQUEST_BODY_BYTES + 1];
    let oversized = send_request(address, "POST", "/uppercase", &oversized_body).await;
    assert!(oversized.starts_with("HTTP/1.1 413 Payload Too Large\r\n"));

    shutdown_tx
        .send(())
        .expect("HTTP shutdown receiver should remain open");
    server.await.expect("HTTP server task should stop cleanly");
}

async fn send_request(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
) -> String {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("test client should connect");
    let request_head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request_head.as_bytes())
        .await
        .expect("request head should write");
    stream
        .write_all(body)
        .await
        .expect("request body should write");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("response should read");
    String::from_utf8(response).expect("HTTP response should be UTF-8")
}

fn response_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("response should contain a header terminator")
}

fn workspace_relative(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join(path)
    }
}
