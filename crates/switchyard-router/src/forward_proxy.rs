use std::{io, net::IpAddr, sync::Arc, time::Duration};

use http::Uri;
use router_config::{Listener, ListenerDestination, Protocol, RouteSlotId};
use router_core::{BrowserLookup, RouteEngine};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::timeout,
};

use crate::{ProcessError, constant_time_eq};

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_REQUEST_BODY_BYTES: u64 = 64 * 1024 * 1024;
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) struct ForwardProxyBinding {
    task: JoinHandle<io::Result<()>>,
}

impl ForwardProxyBinding {
    pub(crate) async fn bind(
        listener: Listener,
        engine: Arc<RouteEngine>,
        shutdown: watch::Receiver<bool>,
    ) -> Result<Self, ProcessError> {
        let authentication = listener.proxy_authentication.as_ref().ok_or_else(|| {
            ProcessError::Configuration(
                "managed forward-proxy listeners require proxy authentication".into(),
            )
        })?;
        let authorization = router_pingora::proxy_authorization(authentication)?;
        let address = (listener.bind.host, listener.bind.port);
        let socket = TcpListener::bind(address).await?;
        let task = tokio::spawn(run(socket, listener, engine, authorization, shutdown));
        Ok(Self { task })
    }

    pub(crate) async fn wait(self) -> Result<(), ProcessError> {
        match self.task.await {
            Ok(result) => result.map_err(Into::into),
            Err(error) if error.is_cancelled() => Ok(()),
            Err(error) => Err(ProcessError::Configuration(error.to_string())),
        }
    }
}

async fn run(
    socket: TcpListener,
    listener: Listener,
    engine: Arc<RouteEngine>,
    authorization: Box<[u8]>,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let listener = Arc::new(listener);
    let authorization = Arc::<[u8]>::from(authorization);
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    connections.abort_all();
                    while connections.join_next().await.is_some() {}
                    return Ok(());
                }
            }
            joined = connections.join_next(), if !connections.is_empty() => {
                let _ = joined;
            }
            accepted = socket.accept() => {
                let (stream, _) = accepted?;
                let listener = Arc::clone(&listener);
                let engine = Arc::clone(&engine);
                let authorization = Arc::clone(&authorization);
                connections.spawn(async move {
                    let _ = serve(stream, &listener, &engine, &authorization).await;
                });
            }
        }
    }
}

struct ParsedRequest {
    upstream_head: Vec<u8>,
    buffered_body: Vec<u8>,
    remaining_body: u64,
    destination: RouteSlotId,
}

struct ProxyFailure {
    status: u16,
    reason: &'static str,
    message: &'static str,
    challenge: bool,
}

impl ProxyFailure {
    fn bad_request(message: &'static str) -> Self {
        Self {
            status: 400,
            reason: "Bad Request",
            message,
            challenge: false,
        }
    }
}

async fn serve(
    mut client: TcpStream,
    listener: &Listener,
    engine: &RouteEngine,
    expected_authorization: &[u8],
) -> io::Result<()> {
    let parsed = match timeout(
        HEADER_TIMEOUT,
        parse_request(&mut client, listener, expected_authorization),
    )
    .await
    {
        Ok(Ok(request)) => request,
        Ok(Err(failure)) => return write_failure(&mut client, failure).await,
        Err(_) => {
            return write_failure(
                &mut client,
                ProxyFailure {
                    status: 408,
                    reason: "Request Timeout",
                    message: "request headers timed out",
                    challenge: false,
                },
            )
            .await;
        }
    };

    let snapshot = engine.snapshot();
    let identity = listener
        .proxy_identity
        .as_ref()
        .expect("validated managed proxy identity");
    let identity = RouteSlotId::from(identity.as_str());
    let target = match snapshot.lookup_browser(BrowserLookup {
        destination: &parsed.destination,
        explicit_header: None,
        origin: None,
        proxy_listener: Some(&identity),
    }) {
        Ok(target) => target.clone(),
        Err(_) => {
            return write_failure(
                &mut client,
                ProxyFailure {
                    status: 403,
                    reason: "Forbidden",
                    message: "managed proxy route is not configured",
                    challenge: false,
                },
            )
            .await;
        }
    };
    if target.endpoint.protocol != Protocol::Http
        || target.endpoint.port == 0
        || !is_loopback_host(&target.endpoint.host)
    {
        return write_failure(
            &mut client,
            ProxyFailure {
                status: 502,
                reason: "Bad Gateway",
                message: "managed proxy upstream is not local HTTP",
                challenge: false,
            },
        )
        .await;
    }
    let address = format!("{}:{}", target.endpoint.host, target.endpoint.port);
    let mut upstream = match timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await {
        Ok(Ok(stream)) => stream,
        _ => {
            return write_failure(
                &mut client,
                ProxyFailure {
                    status: 502,
                    reason: "Bad Gateway",
                    message: "managed proxy upstream is unavailable",
                    challenge: false,
                },
            )
            .await;
        }
    };
    upstream.write_all(&parsed.upstream_head).await?;
    upstream.write_all(&parsed.buffered_body).await?;
    if parsed.remaining_body > 0 {
        let mut body = (&mut client).take(parsed.remaining_body);
        match timeout(TRANSFER_TIMEOUT, tokio::io::copy(&mut body, &mut upstream)).await {
            Ok(Ok(count)) if count == parsed.remaining_body => {}
            _ => return Ok(()),
        }
    }
    upstream.shutdown().await?;
    let _ = timeout(
        TRANSFER_TIMEOUT,
        tokio::io::copy(&mut upstream, &mut client),
    )
    .await;
    let _ = client.shutdown().await;
    Ok(())
}

async fn parse_request(
    client: &mut TcpStream,
    listener: &Listener,
    expected_authorization: &[u8],
) -> Result<ParsedRequest, ProxyFailure> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(ProxyFailure {
                status: 431,
                reason: "Request Header Fields Too Large",
                message: "request headers exceed the managed proxy limit",
                challenge: false,
            });
        }
        let mut chunk = [0_u8; 4096];
        let count = client
            .read(&mut chunk)
            .await
            .map_err(|_| ProxyFailure::bad_request("could not read request"))?;
        if count == 0 {
            return Err(ProxyFailure::bad_request("incomplete request headers"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(ProxyFailure {
            status: 431,
            reason: "Request Header Fields Too Large",
            message: "request headers exceed the managed proxy limit",
            challenge: false,
        });
    }
    let head = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| ProxyFailure::bad_request("request headers must be valid ASCII"))?;
    let mut lines = head[..head.len() - 4].split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| ProxyFailure::bad_request("missing request line"))?;
    let mut parts = request_line.split(' ');
    let method = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProxyFailure::bad_request("invalid request line"))?;
    if !method
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
    {
        return Err(ProxyFailure::bad_request("invalid request method"));
    }
    let target = parts
        .next()
        .ok_or_else(|| ProxyFailure::bad_request("invalid request line"))?;
    let version = parts
        .next()
        .filter(|value| matches!(*value, "HTTP/1.0" | "HTTP/1.1"))
        .ok_or_else(|| ProxyFailure::bad_request("only HTTP/1.x is supported"))?;
    if parts.next().is_some() || method.eq_ignore_ascii_case("CONNECT") {
        return Err(ProxyFailure::bad_request(
            "CONNECT and malformed request targets are not supported",
        ));
    }
    let uri = target
        .parse::<Uri>()
        .map_err(|_| ProxyFailure::bad_request("proxy requests require an absolute HTTP URI"))?;
    if uri.scheme_str() != Some("http") {
        return Err(ProxyFailure::bad_request(
            "managed proxy requests require an absolute http:// URI",
        ));
    }
    let authority = uri
        .authority()
        .ok_or_else(|| ProxyFailure::bad_request("absolute URI has no authority"))?;
    if authority.as_str().contains('@') {
        return Err(ProxyFailure::bad_request("URI credentials are not allowed"));
    }
    let request_target = authority_target(authority)
        .ok_or_else(|| ProxyFailure::bad_request("absolute URI authority is invalid"))?;

    let mut authorization = None;
    let mut host = None;
    let mut content_length = None;
    let mut forwarded = Vec::new();
    for line in lines {
        if line.starts_with([' ', '\t']) {
            return Err(ProxyFailure::bad_request("folded headers are not allowed"));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| ProxyFailure::bad_request("malformed request header"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            return Err(ProxyFailure::bad_request("invalid request header name"));
        }
        let value = value.trim();
        if value
            .bytes()
            .any(|byte| byte == 0x7f || (byte < 0x20 && byte != b'\t'))
        {
            return Err(ProxyFailure::bad_request("invalid request header value"));
        }
        if name.eq_ignore_ascii_case("proxy-authorization") {
            if authorization.replace(value.as_bytes()).is_some() {
                return Err(ProxyFailure::bad_request(
                    "proxy authorization must occur exactly once",
                ));
            }
        } else if name.eq_ignore_ascii_case("host") {
            if host.replace(value).is_some() {
                return Err(ProxyFailure::bad_request("Host must occur exactly once"));
            }
        } else if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(ProxyFailure::bad_request(
                    "Content-Length must occur at most once",
                ));
            }
            content_length = Some(
                value
                    .parse::<u64>()
                    .map_err(|_| ProxyFailure::bad_request("invalid Content-Length"))?,
            );
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(ProxyFailure::bad_request(
                "chunked proxy requests are not supported",
            ));
        } else if !matches!(
            name.to_ascii_lowercase().as_str(),
            "connection" | "proxy-connection" | "keep-alive" | "te" | "trailer" | "upgrade"
        ) {
            forwarded.push((name, value));
        }
    }
    let Some(supplied) = authorization else {
        return Err(ProxyFailure {
            status: 407,
            reason: "Proxy Authentication Required",
            message: "valid managed proxy credentials are required",
            challenge: true,
        });
    };
    if !constant_time_eq(supplied, expected_authorization) {
        return Err(ProxyFailure {
            status: 407,
            reason: "Proxy Authentication Required",
            message: "valid managed proxy credentials are required",
            challenge: true,
        });
    }
    let host = host.ok_or_else(|| ProxyFailure::bad_request("Host is required"))?;
    let host_uri = format!("http://{host}/")
        .parse::<Uri>()
        .map_err(|_| ProxyFailure::bad_request("Host is invalid"))?;
    if host_uri
        .authority()
        .is_some_and(|authority| authority.as_str().contains('@'))
    {
        return Err(ProxyFailure::bad_request(
            "Host credentials are not allowed",
        ));
    }
    let host_target = authority_target(
        host_uri
            .authority()
            .ok_or_else(|| ProxyFailure::bad_request("Host is invalid"))?,
    )
    .ok_or_else(|| ProxyFailure::bad_request("Host is invalid"))?;
    if host_target != request_target {
        return Err(ProxyFailure::bad_request(
            "Host must match the absolute URI authority",
        ));
    }
    let destinations = listener
        .destinations
        .iter()
        .filter(|destination| match destination {
            ListenerDestination::ProxyTarget { host, port, .. } => {
                host.trim_end_matches('.')
                    .eq_ignore_ascii_case(&request_target.0)
                    && *port == request_target.1
            }
            ListenerDestination::CustomDomain { .. }
            | ListenerDestination::LegacyLocalhost { .. }
            | ListenerDestination::Loopback { .. } => false,
        })
        .collect::<Vec<_>>();
    if destinations.len() != 1 {
        return Err(ProxyFailure {
            status: 403,
            reason: "Forbidden",
            message: "absolute URI does not match exactly one managed destination",
            challenge: false,
        });
    }
    let body_length = content_length.unwrap_or(0);
    if body_length > MAX_REQUEST_BODY_BYTES {
        return Err(ProxyFailure {
            status: 413,
            reason: "Content Too Large",
            message: "request body exceeds the managed proxy limit",
            challenge: false,
        });
    }
    let buffered_body = bytes[header_end..].to_vec();
    if buffered_body.len() as u64 > body_length {
        return Err(ProxyFailure::bad_request(
            "pipelined proxy requests are not supported",
        ));
    }

    let path = uri.path_and_query().map_or("/", |value| value.as_str());
    let mut upstream_head = format!("{method} {path} {version}\r\n").into_bytes();
    for (name, value) in forwarded {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        upstream_head.extend_from_slice(name.as_bytes());
        upstream_head.extend_from_slice(b": ");
        upstream_head.extend_from_slice(value.as_bytes());
        upstream_head.extend_from_slice(b"\r\n");
    }
    upstream_head.extend_from_slice(format!("Host: {}\r\n", authority.as_str()).as_bytes());
    if content_length.is_some() {
        upstream_head.extend_from_slice(format!("Content-Length: {body_length}\r\n").as_bytes());
    }
    upstream_head.extend_from_slice(b"Connection: close\r\n\r\n");
    Ok(ParsedRequest {
        upstream_head,
        remaining_body: body_length - buffered_body.len() as u64,
        buffered_body,
        destination: destinations[0].slot().clone(),
    })
}

fn authority_target(authority: &http::uri::Authority) -> Option<(String, u16)> {
    let text = authority.as_str();
    let explicit_port = if text.starts_with('[') {
        let end = text.find(']')?;
        let suffix = &text[end + 1..];
        if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':')?)
        }
    } else {
        text.rsplit_once(':').map(|(_, port)| port)
    };
    let port = match explicit_port {
        Some(port) => port.parse::<u16>().ok()?,
        None => 80,
    };
    if port == 0 {
        return None;
    }
    let host = authority.host();
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    local.then_some((host, port))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

async fn write_failure(client: &mut TcpStream, failure: ProxyFailure) -> io::Result<()> {
    let challenge = if failure.challenge {
        "Proxy-Authenticate: Basic realm=\"switchyard\"\r\n"
    } else {
        ""
    };
    let body = failure.message.as_bytes();
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        failure.status,
        failure.reason,
        body.len(),
        challenge
    );
    client.write_all(response.as_bytes()).await?;
    client.write_all(body).await?;
    client.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_invalid_ports_never_fall_back_to_http_port_80() {
        assert_eq!(
            authority_target(&"localhost".parse().unwrap()),
            Some(("localhost".into(), 80))
        );
        for invalid in ["localhost:99999", "localhost:abc", "localhost:0"] {
            let authority = invalid.parse().unwrap();
            assert_eq!(authority_target(&authority), None, "{invalid}");
            assert_ne!(
                authority_target(&authority),
                Some(("localhost".into(), 80)),
                "{invalid} must not match a declared localhost:80 target"
            );
        }
    }
}
