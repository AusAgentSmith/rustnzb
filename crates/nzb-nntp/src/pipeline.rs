//! Request pipelining for NNTP connections.
//!
//! NNTP responses are strictly ordered, so we can send multiple ARTICLE
//! commands before reading responses. This dramatically improves throughput
//! on high-latency links.

use std::collections::VecDeque;

use tracing::trace;

use crate::connection::{ConnectionState, NntpConnection, NntpResponse};
use crate::error::{NntpError, NntpResult};

// ---------------------------------------------------------------------------
// Pipeline request
// ---------------------------------------------------------------------------

/// A request that has been sent and is awaiting a response.
#[derive(Debug, Clone)]
pub struct PipelineRequest {
    /// The message-id that was requested.
    pub message_id: String,
    /// An opaque tag the caller can use to correlate requests.
    pub tag: u64,
}

/// The result for one pipelined article fetch.
#[derive(Debug)]
pub struct PipelineResult {
    /// The original request.
    pub request: PipelineRequest,
    /// The fetch outcome.
    pub result: NntpResult<NntpResponse>,
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

/// Pipelined NNTP command sender/receiver.
///
/// Usage:
/// 1. Call `submit()` to queue article requests.
/// 2. Internally, up to `depth` ARTICLE commands are sent before any
///    responses are read.
/// 3. Call `receive_one()` to read the next response.
/// 4. Call `drain()` to read all outstanding responses.
pub struct Pipeline {
    /// Maximum number of in-flight requests.
    depth: usize,
    /// Requests that have been sent but whose responses have not been read.
    in_flight: VecDeque<PipelineRequest>,
    /// Requests queued locally but not yet sent to the server.
    pending: VecDeque<PipelineRequest>,
}

impl Pipeline {
    /// Create a new pipeline with the given depth (from `ServerConfig::pipelining`).
    /// A depth of 0 or 1 means no pipelining (send one, read one).
    pub fn new(depth: u8) -> Self {
        let depth = (depth as usize).max(1);
        Self {
            depth,
            in_flight: VecDeque::with_capacity(depth),
            pending: VecDeque::new(),
        }
    }

    /// Queue an article fetch request.
    pub fn submit(&mut self, message_id: String, tag: u64) {
        self.pending.push_back(PipelineRequest { message_id, tag });
    }

    /// Number of requests that have been sent but not yet received.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// Number of requests waiting to be sent.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// True if there are no pending or in-flight requests.
    pub fn is_empty(&self) -> bool {
        self.in_flight.is_empty() && self.pending.is_empty()
    }

    /// Send as many pending requests as the pipeline depth allows.
    ///
    /// This only sends the ARTICLE commands; it does NOT read any responses.
    pub async fn flush_sends(&mut self, conn: &mut NntpConnection) -> NntpResult<()> {
        while self.in_flight.len() < self.depth {
            let Some(req) = self.pending.pop_front() else {
                break;
            };

            let mid = if req.message_id.starts_with('<') {
                req.message_id.clone()
            } else {
                format!("<{}>", req.message_id)
            };

            conn.send_command(&format!("ARTICLE {mid}")).await?;
            trace!(mid = %mid, tag = req.tag, "Pipeline sent ARTICLE");
            self.in_flight.push_back(req);
        }
        Ok(())
    }

    /// Read one response from the server, matching it to the oldest in-flight
    /// request. Returns `None` if there are no in-flight requests.
    pub async fn receive_one(
        &mut self,
        conn: &mut NntpConnection,
    ) -> NntpResult<Option<PipelineResult>> {
        let Some(request) = self.in_flight.pop_front() else {
            return Ok(None);
        };

        let status = conn.read_response_line().await?;

        let result = match status.code {
            220 => {
                // Article follows — read multi-line body
                match conn.read_multiline_body().await {
                    Ok(data) => Ok(NntpResponse {
                        code: status.code,
                        message: status.message,
                        data: Some(data),
                    }),
                    Err(e) => Err(e),
                }
            }
            430 => Err(NntpError::ArticleNotFound(request.message_id.clone())),
            411 => Err(NntpError::NoSuchGroup(status.message)),
            412 | 420 => Err(NntpError::NoArticleSelected(status.message)),
            480 => {
                conn.state = ConnectionState::Error;
                Err(NntpError::AuthRequired(status.message))
            }
            481 | 482 => {
                conn.state = ConnectionState::Error;
                Err(NntpError::Auth(format!(
                    "ARTICLE rejected ({}): {}",
                    status.code, status.message
                )))
            }
            502 => {
                conn.state = ConnectionState::Error;
                Err(NntpError::ServiceUnavailable(status.message))
            }
            _ => {
                conn.state = ConnectionState::Error;
                Err(NntpError::Protocol(format!(
                    "Unexpected ARTICLE response {}: {}",
                    status.code, status.message
                )))
            }
        };

        Ok(Some(PipelineResult { request, result }))
    }

    /// Convenience: submit requests, flush sends, and read all responses.
    ///
    /// This interleaves sending and receiving to keep the pipeline full.
    pub async fn process_all(
        &mut self,
        conn: &mut NntpConnection,
    ) -> NntpResult<Vec<PipelineResult>> {
        let mut results = Vec::with_capacity(self.pending.len() + self.in_flight.len());

        loop {
            // Fill the pipeline
            self.flush_sends(conn).await?;

            if self.in_flight.is_empty() {
                break;
            }

            // Read one response
            if let Some(result) = self.receive_one(conn).await? {
                // If the connection entered an error state, bail out
                let is_fatal = matches!(
                    &result.result,
                    Err(NntpError::Auth(_))
                        | Err(NntpError::AuthRequired(_))
                        | Err(NntpError::ServiceUnavailable(_))
                        | Err(NntpError::Connection(_))
                        | Err(NntpError::Io(_))
                );
                results.push(result);
                if is_fatal {
                    // Drain remaining in-flight as errors
                    while let Some(req) = self.in_flight.pop_front() {
                        results.push(PipelineResult {
                            request: req,
                            result: Err(NntpError::Connection(
                                "Pipeline aborted due to fatal error".into(),
                            )),
                        });
                    }
                    // Move pending back as errors too
                    while let Some(req) = self.pending.pop_front() {
                        results.push(PipelineResult {
                            request: req,
                            result: Err(NntpError::Connection(
                                "Pipeline aborted due to fatal error".into(),
                            )),
                        });
                    }
                    break;
                }
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{MockConfig, MockNntpServer, test_config};
    use std::collections::HashMap;

    #[test]
    fn test_pipeline_submit_and_counts() {
        let mut pipe = Pipeline::new(5);
        assert!(pipe.is_empty());

        pipe.submit("abc@example.com".into(), 1);
        pipe.submit("def@example.com".into(), 2);

        assert_eq!(pipe.pending_count(), 2);
        assert_eq!(pipe.in_flight_count(), 0);
        assert!(!pipe.is_empty());
    }

    #[test]
    fn test_pipeline_depth_minimum() {
        let pipe = Pipeline::new(0);
        assert_eq!(pipe.depth, 1);
    }

    #[test]
    fn test_pipeline_depth_values() {
        assert_eq!(Pipeline::new(1).depth, 1);
        assert_eq!(Pipeline::new(5).depth, 5);
        assert_eq!(Pipeline::new(10).depth, 10);
    }

    #[tokio::test]
    async fn test_pipeline_flush_sends() {
        let mut articles = HashMap::new();
        articles.insert("p1@test".into(), b"data1".to_vec());
        articles.insert("p2@test".into(), b"data2".to_vec());
        articles.insert("p3@test".into(), b"data3".to_vec());

        let server = MockNntpServer::start(MockConfig {
            articles,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let mut pipe = Pipeline::new(2); // depth = 2
        pipe.submit("p1@test".into(), 1);
        pipe.submit("p2@test".into(), 2);
        pipe.submit("p3@test".into(), 3);

        // flush_sends should send up to depth (2) commands
        pipe.flush_sends(&mut conn).await.unwrap();
        assert_eq!(pipe.in_flight_count(), 2);
        assert_eq!(pipe.pending_count(), 1); // p3 still pending
    }

    #[tokio::test]
    async fn test_pipeline_receive_one_success() {
        let mut articles = HashMap::new();
        articles.insert("r1@test".into(), b"response data".to_vec());

        let server = MockNntpServer::start(MockConfig {
            articles,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let mut pipe = Pipeline::new(5);
        pipe.submit("r1@test".into(), 42);
        pipe.flush_sends(&mut conn).await.unwrap();

        let result = pipe.receive_one(&mut conn).await.unwrap().unwrap();
        assert_eq!(result.request.tag, 42);
        assert!(result.result.is_ok());
        let data = result.result.unwrap().data.unwrap();
        assert!(String::from_utf8_lossy(&data).contains("response data"));
    }

    #[tokio::test]
    async fn test_pipeline_receive_one_not_found() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let mut pipe = Pipeline::new(5);
        pipe.submit("missing@test".into(), 99);
        pipe.flush_sends(&mut conn).await.unwrap();

        let result = pipe.receive_one(&mut conn).await.unwrap().unwrap();
        assert_eq!(result.request.tag, 99);
        assert!(matches!(
            result.result,
            Err(crate::error::NntpError::ArticleNotFound(_))
        ));
    }

    #[tokio::test]
    async fn test_pipeline_receive_one_empty() {
        let server = MockNntpServer::start(MockConfig::default()).await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let mut pipe = Pipeline::new(5);
        // No requests submitted — receive returns None
        let result = pipe.receive_one(&mut conn).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_pipeline_process_all() {
        let mut articles = HashMap::new();
        articles.insert("pa1@test".into(), b"data-a".to_vec());
        articles.insert("pa2@test".into(), b"data-b".to_vec());

        let server = MockNntpServer::start(MockConfig {
            articles,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let mut pipe = Pipeline::new(2);
        pipe.submit("pa1@test".into(), 1);
        pipe.submit("pa2@test".into(), 2);

        let results = pipe.process_all(&mut conn).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].result.is_ok());
        assert!(results[1].result.is_ok());
        assert!(pipe.is_empty());
    }

    #[tokio::test]
    async fn test_pipeline_process_all_mixed() {
        let mut articles = HashMap::new();
        articles.insert("hit@test".into(), b"found".to_vec());

        let server = MockNntpServer::start(MockConfig {
            articles,
            ..MockConfig::default()
        })
        .await;
        let config = test_config(server.port());
        let mut conn = NntpConnection::new("test".into());
        conn.connect(&config).await.unwrap();

        let mut pipe = Pipeline::new(5);
        pipe.submit("hit@test".into(), 1);
        pipe.submit("miss@test".into(), 2);
        pipe.submit("hit@test".into(), 3);

        let results = pipe.process_all(&mut conn).await.unwrap();
        assert_eq!(results.len(), 3);
        assert!(results[0].result.is_ok());
        assert!(matches!(
            results[1].result,
            Err(crate::error::NntpError::ArticleNotFound(_))
        ));
        assert!(results[2].result.is_ok());
    }
}
