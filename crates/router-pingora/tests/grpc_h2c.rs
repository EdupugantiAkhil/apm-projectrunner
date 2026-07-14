use std::{net::SocketAddr, sync::Arc, time::Duration};

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Request, Response};
use router_config::RouterConfig;
use router_core::RouteEngine;
use router_pingora::{HttpDataPlane, ProxyOptions};
use serde_json::json;
use tokio::{net::TcpListener, runtime::Builder};

fn config(proxy_port: u16, upstream_port: u16) -> RouterConfig {
    serde_json::from_value(json!({
        "apiVersion": "switchyard.dev/router/v1alpha1",
        "kind": "RouterConfiguration",
        "metadata": { "deployment": "grpc-test" },
        "spec": {
            "snapshot": {
                "id": "grpc-test-1", "version": 1,
                "transitions": {
                    "http": { "strategy": "close" },
                    "https": { "strategy": "close" },
                    "websocket": { "strategy": "close" },
                    "grpc": { "strategy": "drain", "timeoutMs": 1000 },
                    "tcp": { "strategy": "close" }
                }
            },
            "listeners": [{
                "consumer": "grpc-client",
                "bind": { "host": "127.0.0.1", "port": proxy_port },
                "protocol": "grpc",
                "destinations": [{ "kind": "loopback", "slot": "greeter" }]
            }],
            "providers": [{
                "id": "grpc-upstream",
                "endpoint": { "protocol": "grpc", "host": "127.0.0.1", "port": upstream_port }
            }],
            "routes": [{ "consumer": "grpc-client", "slot": "greeter", "provider": "grpc-upstream" }]
        }
    }))
    .unwrap()
}

#[test]
fn passes_grpc_data_and_trailers_over_h2c() {
    Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream_port = upstream.local_addr().unwrap().port();
            let upstream_task = tokio::spawn(async move {
                let (stream, _) = upstream.accept().await.unwrap();
                let mut connection = h2::server::handshake(stream).await.unwrap();
                while let Some(request) = connection.accept().await {
                    tokio::spawn(async move {
                        let (mut request, mut respond) = request.unwrap();
                        let mut received = Vec::new();
                        while let Some(chunk) = request.body_mut().data().await {
                            received.extend_from_slice(&chunk.unwrap());
                        }
                        assert_eq!(received, b"grpc request");
                        let response = Response::builder()
                            .status(200)
                            .header("content-type", "application/grpc")
                            .body(())
                            .unwrap();
                        let mut body = respond.send_response(response, false).unwrap();
                        body.send_data(Bytes::from_static(b"grpc response"), false)
                            .unwrap();
                        let mut trailers = HeaderMap::new();
                        trailers.insert("grpc-status", HeaderValue::from_static("0"));
                        body.send_trailers(trailers).unwrap();
                    });
                }
            });

            let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let proxy_port = reserved.local_addr().unwrap().port();
            drop(reserved);
            let config = config(proxy_port, upstream_port);
            let running = HttpDataPlane::new(
                Arc::new(RouteEngine::new(config.clone()).unwrap()),
                config.spec.listeners.clone(),
                config.spec.identity.clone(),
                ProxyOptions::default(),
            )
            .unwrap()
            .spawn()
            .unwrap();
            assert!(running.wait_ready(Duration::from_secs(2)));

            let stream =
                tokio::net::TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], proxy_port)))
                    .await
                    .unwrap();
            let (mut sender, connection) = h2::client::handshake(stream).await.unwrap();
            let client_task = tokio::spawn(async move { connection.await.unwrap() });
            let request = Request::builder()
                .method("POST")
                .uri("http://localhost/switchyard.Greeter/SayHello")
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .header("content-length", "12")
                .body(())
                .unwrap();
            let (response, mut body) = sender.send_request(request, false).unwrap();
            body.send_data(Bytes::from_static(b"grpc request"), true)
                .unwrap();
            let response = response.await.unwrap();
            assert_eq!(response.status(), 200);
            let mut body = response.into_body();
            assert_eq!(body.data().await.unwrap().unwrap(), "grpc response");
            let trailers = body.trailers().await.unwrap().unwrap();
            assert_eq!(trailers["grpc-status"], "0");

            drop(sender);
            client_task.abort();
            let _ = client_task.await;
            running.shutdown();
            upstream_task.abort();
            let _ = upstream_task.await;
        });
}
