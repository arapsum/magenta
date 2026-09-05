use std::time::Duration;

use futures_util::io::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClient, HttpRequestExt as _, Method, Request, Response};

const BODY_READ_CHUNK: usize = 8192;

#[derive(Debug, thiserror::Error)]
pub enum HttpBuildError {
    #[error("could not build the HTTP request: {0}")]
    Build(String),
}

pub fn request(
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
    timeout: Duration,
) -> Result<Request<AsyncBody>, HttpBuildError> {
    let mut builder = Request::builder().method(method).uri(uri).timeout(timeout);

    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }

    builder
        .body(AsyncBody::from(body))
        .map_err(|error| HttpBuildError::Build(error.to_string()))
}

pub async fn send(
    client: &dyn HttpClient,
    request: Request<AsyncBody>,
) -> Result<Response<AsyncBody>, String> {
    client
        .send(request)
        .await
        .map_err(|error| error.to_string())
}

pub async fn read_limited(body: &mut AsyncBody, limit: usize) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::with_capacity(limit.min(BODY_READ_CHUNK));
    let mut chunk = [0; BODY_READ_CHUNK];

    while bytes.len() < limit {
        let remaining = limit - bytes.len();
        let read_size = remaining.min(chunk.len());
        let read = body.read(&mut chunk[..read_size]).await?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }

    Ok(bytes)
}
