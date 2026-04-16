use api::ApiError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn reqwest_decode_errors_are_retryable() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let body = b"{not_valid_json";
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(headers.as_bytes()).await;
            let _ = socket.write_all(body).await;
            let _ = socket.shutdown().await;
        }
    });

    let client = reqwest::Client::new();
    let reqwest_err = client
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect("send should reach server")
        .json::<serde_json::Value>()
        .await
        .expect_err("malformed body must fail to decode as JSON");

    assert!(
        reqwest_err.is_decode(),
        "expected a decode-kind reqwest error, got {reqwest_err:?}"
    );
    let api_err: ApiError = reqwest_err.into();
    assert!(
        api_err.is_retryable(),
        "decode-kind Http errors must be retryable: {api_err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn reqwest_body_errors_are_retryable() {
    // Advertise a long Content-Length then close the connection after sending
    // only a prefix. reqwest surfaces this as an `is_body()` error when the
    // caller drains the body.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            let headers = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 1024\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(headers).await;
            let _ = socket.write_all(b"truncated").await;
            // Drop without flushing the remaining 1015 bytes.
        }
    });

    let client = reqwest::Client::new();
    let reqwest_err = client
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect("send should reach server")
        .bytes()
        .await
        .expect_err("short body must fail to drain fully");

    assert!(
        reqwest_err.is_body() || reqwest_err.is_decode(),
        "expected a body/decode-kind reqwest error, got {reqwest_err:?}"
    );
    let api_err: ApiError = reqwest_err.into();
    assert!(
        api_err.is_retryable(),
        "mid-body transport errors must be retryable: {api_err}"
    );
}
