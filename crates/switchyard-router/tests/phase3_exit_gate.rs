use std::{
    fs,
    net::{SocketAddr, TcpListener as StdTcpListener},
    os::unix::fs::PermissionsExt,
    time::Duration,
};

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Request, Response};
use router_config::RouterConfig;
use serde_json::json;
use switchyard_router::{
    AdminOptions, RouterProcess,
    host_gateway::{ensure_proxy_credentials, preflight},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::{sleep, timeout},
};

const IO_TIMEOUT: Duration = Duration::from_secs(3);

struct IdentityUpstream {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl IdentityUpstream {
    async fn start(identity: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let Ok(request) = read_head(&mut stream).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                    let body = if request.contains("x-switchyard-route:")
                        || request.contains("proxy-authorization:")
                    {
                        "routing-credential-leaked"
                    } else {
                        identity
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        Self { address, task }
    }
}

impl Drop for IdentityUpstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_unchanged_browser_callers_route_independently_and_fail_closed() {
    let explicit_upstream = IdentityUpstream::start("explicit-provider").await;
    let origin_upstream = IdentityUpstream::start("origin-provider").await;
    let profile_upstream = IdentityUpstream::start("profile-provider").await;
    let profile_ui_upstream = IdentityUpstream::start("profile-ui-provider").await;
    let browser_port = unused_port();
    let profile_port = unused_port();
    let directory = tempfile::tempdir().unwrap();
    let credential = directory.path().join("profile.credential");
    fs::write(&credential, "profile-token").unwrap();
    fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();

    let config: RouterConfig = serde_json::from_value(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": "phase3-browser-gate" },
        "spec": {
            "snapshot": {
                "id": "phase3-browser-gate-1", "version": 1,
                "transitions": transitions()
            },
            "listeners": [
                {
                    "bind": { "host": "127.0.0.1", "port": browser_port },
                    "protocol": "http",
                    "destinations": [
                        { "kind": "legacy_localhost", "slot": "browser-backend", "host": "localhost" },
                        { "kind": "custom_domain", "slot": "profile-ui", "domain": "ui-three.localhost" }
                    ]
                },
                {
                    "bind": { "host": "127.0.0.1", "port": profile_port },
                    "protocol": "http",
                    "destinations": [
                        { "kind": "proxy_target", "slot": "browser-backend", "host": "localhost", "port": browser_port },
                        { "kind": "proxy_target", "slot": "profile-ui", "host": "ui-three.localhost", "port": browser_port }
                    ],
                    "proxyIdentity": "profile-three",
                    "proxyAuthentication": {
                        "scheme": "basic", "credentialFile": credential
                    }
                }
            ],
            "providers": [
                {
                    "id": "explicit-provider",
                    "endpoint": {
                        "protocol": "http", "host": "127.0.0.1", "port": explicit_upstream.address.port()
                    }
                },
                {
                    "id": "origin-provider",
                    "endpoint": {
                        "protocol": "http", "host": "127.0.0.1", "port": origin_upstream.address.port()
                    }
                },
                {
                    "id": "profile-provider",
                    "endpoint": {
                        "protocol": "http", "host": "127.0.0.1", "port": profile_upstream.address.port()
                    }
                },
                {
                    "id": "profile-ui-provider",
                    "endpoint": {
                        "protocol": "http", "host": "127.0.0.1", "port": profile_ui_upstream.address.port()
                    }
                }
            ],
            "groups": [], "bindings": [], "routes": [],
            "browserRoutes": [
                {
                    "identity": { "source": "explicit_header", "value": "tab-one" },
                    "destination": "browser-backend", "provider": "explicit-provider"
                },
                {
                    "identity": { "source": "explicit_header", "value": "tab-two" },
                    "destination": "browser-backend", "provider": "origin-provider"
                },
                {
                    "identity": { "source": "origin", "origin": "https://ui-two.test" },
                    "destination": "browser-backend", "provider": "origin-provider"
                },
                {
                    "identity": { "source": "proxy_listener", "listener": "profile-three" },
                    "destination": "browser-backend", "provider": "profile-provider"
                },
                {
                    "identity": { "source": "proxy_listener", "listener": "profile-three" },
                    "destination": "profile-ui", "provider": "profile-ui-provider"
                }
            ],
            "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
        }
    }))
    .unwrap();

    preflight(&config).unwrap();
    assert!(ensure_proxy_credentials(&config).unwrap().is_empty());
    let process = RouterProcess::start(
        config,
        AdminOptions {
            socket_path: directory.path().join("admin.socket"),
            token: "test-token".into(),
        },
    )
    .await
    .unwrap();

    let browser = SocketAddr::from(([127, 0, 0, 1], browser_port));
    let profile = SocketAddr::from(([127, 0, 0, 1], profile_port));
    assert_eq!(
        response_body(
            &http_request(
                browser,
                "GET /identity HTTP/1.1\r\nHost: localhost\r\nX-Switchyard-Route: tab-one\r\nConnection: close\r\n\r\n",
            )
            .await,
        ),
        "explicit-provider"
    );
    let origin_response = http_request(
        browser,
        "GET /identity HTTP/1.1\r\nHost: localhost\r\nOrigin: https://ui-two.test\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert_eq!(response_body(&origin_response), "origin-provider");
    assert!(
        origin_response
            .to_ascii_lowercase()
            .contains("access-control-allow-origin: https://ui-two.test")
    );
    let profile_ui_response = http_request(
        profile,
        &format!(
            "GET http://ui-three.localhost:{browser_port}/ HTTP/1.1\r\nHost: ui-three.localhost:{browser_port}\r\nProxy-Authorization: Basic c3dpdGNoeWFyZDpwcm9maWxlLXRva2Vu\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert_eq!(
        response_body(&profile_ui_response),
        "profile-ui-provider",
        "{profile_ui_response}"
    );
    let profile_response = http_request(
        profile,
        &format!(
            "GET http://localhost:{browser_port}/identity HTTP/1.1\r\nHost: localhost:{browser_port}\r\nProxy-Authorization: Basic c3dpdGNoeWFyZDpwcm9maWxlLXRva2Vu\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert_eq!(
        response_body(&profile_response),
        "profile-provider",
        "{profile_response}"
    );
    let unauthenticated_profile = http_request(
        profile,
        &format!(
            "GET http://localhost:{browser_port}/identity HTTP/1.1\r\nHost: localhost:{browser_port}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(unauthenticated_profile.starts_with("HTTP/1.1 407"));
    assert!(
        unauthenticated_profile
            .to_ascii_lowercase()
            .contains("proxy-authenticate: basic")
    );
    let connect = http_request(
        profile,
        "CONNECT localhost:443 HTTP/1.1\r\nHost: localhost:443\r\nProxy-Authorization: Basic c3dpdGNoeWFyZDpwcm9maWxlLXRva2Vu\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(connect.starts_with("HTTP/1.1 400"));
    let wrong_port = if browser_port == u16::MAX {
        browser_port - 1
    } else {
        browser_port + 1
    };
    let mismatched_authority = http_request(
        profile,
        &format!(
            "GET http://localhost:{browser_port}/identity HTTP/1.1\r\nHost: localhost:{wrong_port}\r\nProxy-Authorization: Basic c3dpdGNoeWFyZDpwcm9maWxlLXRva2Vu\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(mismatched_authority.starts_with("HTTP/1.1 400"));
    let undeclared_port = http_request(
        profile,
        &format!(
            "GET http://localhost:{wrong_port}/identity HTTP/1.1\r\nHost: localhost:{wrong_port}\r\nProxy-Authorization: Basic c3dpdGNoeWFyZDpwcm9maWxlLXRva2Vu\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(undeclared_port.starts_with("HTTP/1.1 403"));

    let missing = http_request(
        browser,
        "GET /identity HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(missing.starts_with("HTTP/1.1 400"));
    assert!(missing.contains("\"code\":\"missing_route_identity\""));
    let unknown = http_request(
        browser,
        "GET /identity HTTP/1.1\r\nHost: localhost\r\nX-Switchyard-Route: undeclared\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(unknown.starts_with("HTTP/1.1 403"));
    assert!(unknown.contains("\"code\":\"unknown_route_identity\""));

    let mut concurrent = Vec::new();
    for index in 0..24 {
        concurrent.push(tokio::spawn(async move {
            let (route, expected) = if index % 2 == 0 {
                ("tab-one", "explicit-provider")
            } else {
                ("tab-two", "origin-provider")
            };
            let response = http_request(
                browser,
                &format!(
                    "GET /identity HTTP/1.1\r\nHost: localhost\r\nX-Switchyard-Route: {route}\r\nConnection: close\r\n\r\n"
                ),
            )
            .await;
            assert_eq!(response_body(&response), expected);
        }));
    }
    for task in concurrent {
        task.await.unwrap();
    }

    process.request_shutdown();
    process.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_process_preserves_streaming_websocket_grpc_and_raw_tcp() {
    let (http_upstream, http_release, http_task) = streaming_http_upstream().await;
    let (websocket_upstream, websocket_task) = websocket_upstream().await;
    let (grpc_upstream, grpc_release, grpc_task) = grpc_upstream().await;
    let (tcp_upstream, tcp_task) = tcp_upstream().await;
    let http_port = unused_port();
    let websocket_port = unused_port();
    let grpc_port = unused_port();
    let tcp_port = unused_port();
    let directory = tempfile::tempdir().unwrap();

    let config: RouterConfig = serde_json::from_value(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": "phase3-protocol-gate" },
        "spec": {
            "snapshot": {
                "id": "phase3-protocol-gate-1", "version": 1,
                "transitions": transitions()
            },
            "listeners": [
                {
                    "consumer": "host-client", "bind": { "host": "127.0.0.1", "port": http_port },
                    "protocol": "http", "destinations": [{ "kind": "loopback", "slot": "http-stream" }]
                },
                {
                    "consumer": "host-client", "bind": { "host": "127.0.0.1", "port": websocket_port },
                    "protocol": "websocket", "destinations": [{ "kind": "loopback", "slot": "websocket-stream" }]
                },
                {
                    "consumer": "host-client", "bind": { "host": "127.0.0.1", "port": grpc_port },
                    "protocol": "grpc", "destinations": [{ "kind": "loopback", "slot": "grpc-stream" }]
                },
                {
                    "consumer": "host-client", "bind": { "host": "127.0.0.1", "port": tcp_port },
                    "protocol": "tcp", "destinations": [{ "kind": "loopback", "slot": "tcp-stream" }]
                }
            ],
            "providers": [
                {
                    "id": "http-upstream",
                    "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": http_upstream.port() }
                },
                {
                    "id": "websocket-upstream",
                    "endpoint": { "protocol": "websocket", "host": "127.0.0.1", "port": websocket_upstream.port() }
                },
                {
                    "id": "grpc-upstream",
                    "endpoint": { "protocol": "grpc", "host": "127.0.0.1", "port": grpc_upstream.port() }
                },
                {
                    "id": "tcp-upstream",
                    "endpoint": { "protocol": "tcp", "host": "127.0.0.1", "port": tcp_upstream.port() }
                }
            ],
            "groups": [], "bindings": [],
            "routes": [
                { "consumer": "host-client", "slot": "http-stream", "provider": "http-upstream" },
                { "consumer": "host-client", "slot": "websocket-stream", "provider": "websocket-upstream" },
                { "consumer": "host-client", "slot": "grpc-stream", "provider": "grpc-upstream" },
                { "consumer": "host-client", "slot": "tcp-stream", "provider": "tcp-upstream" }
            ],
            "browserRoutes": [],
            "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
        }
    }))
    .unwrap();

    preflight(&config).unwrap();
    let process = RouterProcess::start(
        config,
        AdminOptions {
            socket_path: directory.path().join("admin.socket"),
            token: "test-token".into(),
        },
    )
    .await
    .unwrap();

    let mut http = connect_retry(SocketAddr::from(([127, 0, 0, 1], http_port))).await;
    http.write_all(b"GET /stream HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let first = read_until(&mut http, b"alpha").await;
    assert!(first.windows(5).any(|window| window == b"alpha"));
    assert!(!first.windows(5).any(|window| window == b"omega"));
    http_release.send(()).unwrap();
    let mut remainder = Vec::new();
    timeout(IO_TIMEOUT, http.read_to_end(&mut remainder))
        .await
        .unwrap()
        .unwrap();
    assert!(remainder.windows(5).any(|window| window == b"omega"));

    let mut websocket = connect_retry(SocketAddr::from(([127, 0, 0, 1], websocket_port))).await;
    websocket
        .write_all(b"GET /ws HTTP/1.1\r\nHost: localhost\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n")
        .await
        .unwrap();
    let handshake = read_head(&mut websocket).await.unwrap();
    assert!(String::from_utf8_lossy(&handshake).starts_with("HTTP/1.1 101"));
    websocket.write_all(b"ping").await.unwrap();
    let mut echoed = [0; 4];
    timeout(IO_TIMEOUT, websocket.read_exact(&mut echoed))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&echoed, b"ping");
    drop(websocket);

    let grpc_stream = connect_retry(SocketAddr::from(([127, 0, 0, 1], grpc_port))).await;
    let (mut sender, connection) = timeout(IO_TIMEOUT, h2::client::handshake(grpc_stream))
        .await
        .unwrap()
        .unwrap();
    let grpc_client = tokio::spawn(async move { connection.await.unwrap() });
    let request = Request::builder()
        .method("POST")
        .uri("http://localhost/switchyard.Greeter/Stream")
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .header("content-length", "12")
        .body(())
        .unwrap();
    let (response, mut request_body) = sender.send_request(request, false).unwrap();
    request_body
        .send_data(Bytes::from_static(b"grpc-request"), true)
        .unwrap();
    let response = timeout(IO_TIMEOUT, response).await.unwrap().unwrap();
    assert_eq!(response.status(), 200);
    let mut response_body = response.into_body();
    assert_eq!(
        timeout(IO_TIMEOUT, response_body.data())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "grpc-alpha"
    );
    grpc_release.send(()).unwrap();
    assert_eq!(
        timeout(IO_TIMEOUT, response_body.data())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "grpc-omega"
    );
    let trailers = timeout(IO_TIMEOUT, response_body.trailers())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(trailers["grpc-status"], "0");
    drop(sender);
    grpc_client.abort();

    let mut tcp = connect_retry(SocketAddr::from(([127, 0, 0, 1], tcp_port))).await;
    let mut identity = [0];
    timeout(IO_TIMEOUT, tcp.read_exact(&mut identity))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(identity[0], b'T');
    tcp.write_all(b"raw").await.unwrap();
    let mut raw = [0; 3];
    timeout(IO_TIMEOUT, tcp.read_exact(&mut raw))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&raw, b"raw");

    drop(response_body);
    drop(tcp);
    process.request_shutdown();
    process.wait().await.unwrap();
    http_task.await.unwrap();
    websocket_task.await.unwrap();
    tcp_task.await.unwrap();
    grpc_task.abort();
}

fn transitions() -> serde_json::Value {
    json!({
        "http": { "strategy": "close" },
        "https": { "strategy": "close" },
        "websocket": { "strategy": "pin" },
        "grpc": { "strategy": "drain", "timeoutMs": 1000 },
        "tcp": { "strategy": "close" }
    })
}

fn unused_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn connect_retry(address: SocketAddr) -> TcpStream {
    timeout(IO_TIMEOUT, async {
        loop {
            match TcpStream::connect(address).await {
                Ok(stream) => return stream,
                Err(_) => sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("listener did not become ready")
}

async fn read_head(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    Ok(head)
}

async fn read_until(stream: &mut TcpStream, needle: &[u8]) -> Vec<u8> {
    timeout(IO_TIMEOUT, async {
        let mut bytes = Vec::new();
        let mut chunk = [0; 1024];
        while !bytes.windows(needle.len()).any(|window| window == needle) {
            let count = stream.read(&mut chunk).await.unwrap();
            assert_ne!(count, 0, "connection closed before streamed bytes arrived");
            bytes.extend_from_slice(&chunk[..count]);
        }
        bytes
    })
    .await
    .expect("streamed bytes did not arrive")
}

async fn http_request(address: SocketAddr, request: &str) -> String {
    let mut stream = connect_retry(address).await;
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    timeout(IO_TIMEOUT, stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(response).unwrap()
}

fn response_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("response had no body separator")
}

async fn streaming_http_upstream() -> (SocketAddr, oneshot::Sender<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release, released) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_head(&mut stream).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nalpha\r\n")
            .await
            .unwrap();
        released.await.unwrap();
        stream.write_all(b"5\r\nomega\r\n0\r\n\r\n").await.unwrap();
    });
    (address, release, task)
}

async fn websocket_upstream() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_head(&mut stream).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: websocket\r\n\r\n")
            .await
            .unwrap();
        let mut payload = [0; 4];
        stream.read_exact(&mut payload).await.unwrap();
        stream.write_all(&payload).await.unwrap();
    });
    (address, task)
}

async fn grpc_upstream() -> (SocketAddr, oneshot::Sender<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release, released) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = h2::server::handshake(stream).await.unwrap();
        let (mut request, mut respond) = connection.accept().await.unwrap().unwrap();
        tokio::spawn(async move {
            let mut received = Vec::new();
            while let Some(chunk) = request.body_mut().data().await {
                received.extend_from_slice(&chunk.unwrap());
            }
            assert_eq!(received, b"grpc-request");
            let response = Response::builder()
                .status(200)
                .header("content-type", "application/grpc")
                .body(())
                .unwrap();
            let mut body = respond.send_response(response, false).unwrap();
            body.send_data(Bytes::from_static(b"grpc-alpha"), false)
                .unwrap();
            released.await.unwrap();
            body.send_data(Bytes::from_static(b"grpc-omega"), false)
                .unwrap();
            let mut trailers = HeaderMap::new();
            trailers.insert("grpc-status", HeaderValue::from_static("0"));
            body.send_trailers(trailers).unwrap();
        });
        while connection.accept().await.is_some() {}
    });
    (address, release, task)
}

async fn tcp_upstream() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(b"T").await.unwrap();
        let mut payload = [0; 3];
        stream.read_exact(&mut payload).await.unwrap();
        stream.write_all(&payload).await.unwrap();
    });
    (address, task)
}
