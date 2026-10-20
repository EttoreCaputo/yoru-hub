//! An HTTP client that accepts what Google's MCP servers really send.
//!
//! Gmail answers `tools/list` with a complete JSON-RPC result but the status
//! `403 Forbidden`. The MCP library treats any non-2xx status as a failure and
//! keeps the body only in the error text, so the session would never come up.
//! This wrapper takes that body back when it is a valid JSON-RPC result.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream::BoxStream;
use http::{HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_client::{SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse};
use sse_stream::Sse;

#[derive(Clone)]
pub struct Lenient<C>(pub C);

/// The JSON-RPC result inside a "HTTP <status>: <body>" error, if that is what it carries.
fn result_in_error<E: std::error::Error + Send + Sync + 'static>(e: &StreamableHttpError<E>) -> Option<ServerJsonRpcMessage> {
    let StreamableHttpError::UnexpectedServerResponse(text) = e else { return None };
    let body = text.strip_prefix("HTTP ")?.split_once(": ")?.1;
    match serde_json::from_str::<ServerJsonRpcMessage>(body).ok()? {
        message @ ServerJsonRpcMessage::Response(_) => Some(message),
        _ => None,
    }
}

impl<C: StreamableHttpClient + Sync> StreamableHttpClient for Lenient<C> {
    type Error = C::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        match self.0.post_message(uri, message, session_id, auth_header, custom_headers).await {
            Err(e) => match result_in_error(&e) {
                Some(message) => Ok(StreamableHttpPostResponse::Json(message, None)),
                None => Err(e),
            },
            ok => ok,
        }
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        match self.0.post_message_with_max_sse_event_size(uri, message, session_id, auth_header, custom_headers, max_sse_event_size).await {
            Err(e) => match result_in_error(&e) {
                Some(message) => Ok(StreamableHttpPostResponse::Json(message, None)),
                None => Err(e),
            },
            ok => ok,
        }
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.0.delete_session(uri, session_id, auth_header, custom_headers).await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.0.get_stream(uri, session_id, last_event_id, auth_header, custom_headers).await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.0.get_stream_with_max_sse_event_size(uri, session_id, last_event_id, auth_header, custom_headers, max_sse_event_size).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Err = StreamableHttpError<std::io::Error>;

    fn unexpected(text: &str) -> Err {
        StreamableHttpError::UnexpectedServerResponse(text.to_string().into())
    }

    #[test]
    fn takes_a_result_back_from_a_403() {
        let e = unexpected(r#"HTTP 403 Forbidden: {"id":1,"jsonrpc":"2.0","result":{"tools":[]}}"#);
        assert!(result_in_error(&e).is_some());
    }

    #[test]
    fn leaves_real_errors_alone() {
        assert!(result_in_error(&unexpected(r#"HTTP 403 Forbidden: {"id":1,"jsonrpc":"2.0","error":{"code":-32000,"message":"no"}}"#)).is_none());
        assert!(result_in_error(&unexpected("HTTP 403 Forbidden: <html>denied</html>")).is_none());
        assert!(result_in_error(&unexpected("invalid www-authenticate header value")).is_none());
    }
}
