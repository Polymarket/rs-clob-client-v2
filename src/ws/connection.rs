#![expect(
    clippy::module_name_repetitions,
    reason = "Connection types expose their domain in the name for clarity"
)]

use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use backoff::backoff::Backoff as _;
use futures::{SinkExt as _, StreamExt as _};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{interval, sleep, timeout};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use super::config::Config;
use super::error::WsError;
use super::traits::{MessageParser, ParserDiagnostic, ParserFailureClassification};
use crate::auth::Credentials;
use crate::error::Kind;
use crate::ws::WithCredentials;
use crate::{Result, error::Error};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Broadcast channel capacity for incoming messages.
const BROADCAST_CAPACITY: usize = 1024;

/// Connection state tracking.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Not connected
    Disconnected,
    /// Attempting to connect
    Connecting,
    /// Successfully connected
    Connected {
        /// When the connection was established
        since: Instant,
    },
    /// Reconnecting after failure
    Reconnecting {
        /// Current reconnection attempt number
        attempt: u32,
    },
}

impl ConnectionState {
    /// Check if the connection is currently active.
    #[must_use]
    pub const fn is_connected(self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// Immutable generation identity for one successfully established WebSocket connection.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ConnectionGeneration(pub u64);

impl ConnectionGeneration {
    /// Zero means no connection generation has been established yet.
    #[must_use]
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Return the numeric generation identifier.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// Incoming message tagged with the connection generation that delivered it.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ConnectionEnvelope<M> {
    /// Immutable connection generation.
    pub generation: ConnectionGeneration,
    /// Parsed message payload.
    pub message: M,
}

/// Low-level connection lifecycle or continuity diagnostic.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionDiagnostic {
    /// Generation affected by the diagnostic.
    pub generation: ConnectionGeneration,
    /// Typed diagnostic reason.
    pub kind: ConnectionDiagnosticKind,
}

/// Typed connection lifecycle and continuity reasons.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionDiagnosticKind {
    /// A connection generation was established.
    Connected,
    /// The remote peer closed the socket.
    ConnectionClosed,
    /// The socket failed with a transport error.
    ConnectionError,
    /// A single connection attempt timed out.
    ConnectTimeout {
        /// Attempt number consumed by the timeout.
        attempt: u32,
        /// Configured timeout duration.
        timeout: Duration,
    },
    /// Reconnect policy reached its configured attempt limit.
    ReconnectExhausted {
        /// Number of attempts consumed by the policy.
        attempts: u32,
    },
    /// Heartbeat timed out while waiting for PONG.
    HeartbeatTimeout {
        /// Configured heartbeat timeout.
        timeout: Duration,
    },
    /// A parser failure or bounded parser diagnostic occurred.
    ParserFailure(ParserDiagnostic),
    /// An outbound socket write failed.
    WriteFailed,
    /// Explicit shutdown was requested.
    Shutdown,
}

/// Manages WebSocket connection lifecycle, reconnection, and heartbeat.
///
/// This generic connection manager handles all WebSocket connection concerns:
/// - Establishing and maintaining connections
/// - Automatic reconnection with exponential backoff
/// - Heartbeat monitoring via PING/PONG
/// - Broadcasting messages to multiple subscribers
///
/// # Type Parameters
///
/// - `M`: Message type that implements [`DeserializeOwned`] among other "helper" types
/// - `P`: Parser type that implements [`MessageParser<M>`]
///
/// # Example
///
/// ```ignore
/// let parser = SimpleParser;
/// let connection = ConnectionManager::new(
///     "wss://example.com".to_owned(),
///     config,
///     parser,
/// )?;
///
/// // Subscribe to messages
/// let mut rx = connection.subscribe();
/// while let Ok(msg) = rx.recv().await {
///     println!("Received: {:?}", msg);
/// }
/// ```
#[derive(Clone)]
pub struct ConnectionManager<M, P>
where
    M: DeserializeOwned + Debug + Clone + Send + 'static,
    P: MessageParser<M>,
{
    /// Watch channel sender for state changes (enables reconnection detection)
    state_tx: watch::Sender<ConnectionState>,
    /// Watch channel receiver for state changes (for use in checking the current state)
    state_rx: watch::Receiver<ConnectionState>,
    /// Sender channel for outgoing messages
    sender_tx: mpsc::UnboundedSender<String>,
    /// Broadcast sender for incoming messages
    broadcast_tx: broadcast::Sender<ConnectionEnvelope<M>>,
    /// Broadcast sender for connection and parser diagnostics
    diagnostic_tx: broadcast::Sender<ConnectionDiagnostic>,
    /// Watch sender for current generation
    generation_tx: watch::Sender<ConnectionGeneration>,
    /// Watch receiver for current generation
    generation_rx: watch::Receiver<ConnectionGeneration>,
    /// Signal used to stop pending connect, active loops, heartbeat, and reconnect backoff
    shutdown_tx: watch::Sender<bool>,
    /// Shutdown flag used to reject post-shutdown writes synchronously
    shutdown: Arc<AtomicBool>,
    /// Closed signal for the connection task
    closed_rx: watch::Receiver<bool>,
    /// Join handle for the SDK-owned connection task
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Phantom data for unused type parameters
    _phantom: PhantomData<P>,
}

impl<M, P> ConnectionManager<M, P>
where
    M: DeserializeOwned + Debug + Clone + Send + 'static,
    P: MessageParser<M>,
{
    /// Create a new connection manager and start the connection loop.
    ///
    /// The `parser` is used to deserialize incoming WebSocket messages.
    /// The connection loop runs in a background task and automatically
    /// handles reconnection according to the config's `ReconnectConfig`.
    pub fn new(endpoint: String, config: Config, parser: P) -> Result<Self> {
        let (sender_tx, sender_rx) = mpsc::unbounded_channel();
        let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (diagnostic_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let (generation_tx, generation_rx) = watch::channel(ConnectionGeneration::zero());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let shutdown = Arc::new(AtomicBool::new(false));
        let (closed_tx, closed_rx) = watch::channel(false);

        // Spawn connection task
        let connection_config = config;
        let connection_endpoint = endpoint;
        let broadcast_tx_clone = broadcast_tx.clone();
        let diagnostic_tx_clone = diagnostic_tx.clone();
        let state_tx_clone = state_tx.clone();
        let generation_tx_clone = generation_tx.clone();
        let shutdown_clone = Arc::clone(&shutdown);

        let handle = tokio::spawn(async move {
            Self::connection_loop(
                connection_endpoint,
                connection_config,
                sender_rx,
                broadcast_tx_clone,
                diagnostic_tx_clone,
                parser,
                state_tx_clone,
                generation_tx_clone,
                shutdown_rx,
                shutdown_clone,
                closed_tx,
            )
            .await;
        });

        Ok(Self {
            state_tx,
            state_rx,
            sender_tx,
            broadcast_tx,
            diagnostic_tx,
            generation_tx,
            generation_rx,
            shutdown_tx,
            shutdown,
            closed_rx,
            task: Arc::new(Mutex::new(Some(handle))),
            _phantom: PhantomData,
        })
    }

    /// Main connection loop with automatic reconnection.
    #[expect(
        clippy::too_many_arguments,
        reason = "the spawned loop owns independent channels so shutdown can cancel every task"
    )]
    async fn connection_loop(
        endpoint: String,
        config: Config,
        mut sender_rx: mpsc::UnboundedReceiver<String>,
        broadcast_tx: broadcast::Sender<ConnectionEnvelope<M>>,
        diagnostic_tx: broadcast::Sender<ConnectionDiagnostic>,
        parser: P,
        state_tx: watch::Sender<ConnectionState>,
        generation_tx: watch::Sender<ConnectionGeneration>,
        mut shutdown_rx: watch::Receiver<bool>,
        shutdown: Arc<AtomicBool>,
        closed_tx: watch::Sender<bool>,
    ) {
        let mut attempt = 0_u32;
        let mut backoff: backoff::ExponentialBackoff = config.reconnect.clone().into();
        let mut generation = ConnectionGeneration::zero();

        loop {
            // Check if ConnectionManager was dropped (all sender_tx instances gone)
            if sender_rx.is_closed() || *shutdown_rx.borrow() {
                #[cfg(feature = "tracing")]
                tracing::debug!("Sender channel closed, stopping connection loop");
                _ = state_tx.send(ConnectionState::Disconnected);
                _ = diagnostic_tx.send(ConnectionDiagnostic {
                    generation,
                    kind: ConnectionDiagnosticKind::Shutdown,
                });
                break;
            }

            let state_rx = state_tx.subscribe();

            _ = state_tx.send(ConnectionState::Connecting);

            // Attempt connection
            let connect_result = tokio::select! {
                () = wait_for_shutdown(&mut shutdown_rx) => {
                    _ = state_tx.send(ConnectionState::Disconnected);
                    _ = diagnostic_tx.send(ConnectionDiagnostic {
                        generation,
                        kind: ConnectionDiagnosticKind::Shutdown,
                    });
                    break;
                }
                result = timeout(config.connect_timeout, connect_async(&endpoint)) => result,
            };

            match connect_result {
                Ok(Ok((ws_stream, _))) => {
                    attempt = 0;
                    backoff.reset();
                    generation = generation.next();
                    _ = generation_tx.send(generation);
                    _ = state_tx.send(ConnectionState::Connected {
                        since: Instant::now(),
                    });
                    _ = diagnostic_tx.send(ConnectionDiagnostic {
                        generation,
                        kind: ConnectionDiagnosticKind::Connected,
                    });

                    // Handle connection
                    if let Err(e) = Self::handle_connection(
                        ws_stream,
                        &mut sender_rx,
                        &broadcast_tx,
                        &diagnostic_tx,
                        state_rx,
                        shutdown_rx.clone(),
                        config.clone(),
                        &parser,
                        generation,
                    )
                    .await
                    {
                        #[cfg(feature = "tracing")]
                        tracing::error!("Error handling connection: {e:?}");
                        #[cfg(not(feature = "tracing"))]
                        let _: &_ = &e;
                        attempt = attempt.saturating_add(1);
                    }
                }
                Ok(Err(e)) => {
                    let error = Error::with_source(Kind::WebSocket, WsError::Connection(e));
                    #[cfg(feature = "tracing")]
                    tracing::warn!("Unable to connect: {error:?}");
                    #[cfg(not(feature = "tracing"))]
                    let _: &_ = &error;
                    attempt = attempt.saturating_add(1);
                }
                Err(_) => {
                    attempt = attempt.saturating_add(1);
                    _ = diagnostic_tx.send(ConnectionDiagnostic {
                        generation,
                        kind: ConnectionDiagnosticKind::ConnectTimeout {
                            attempt,
                            timeout: config.connect_timeout,
                        },
                    });
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        attempt,
                        timeout = ?config.connect_timeout,
                        endpoint = %endpoint,
                        "WebSocket connection attempt timed out"
                    );
                }
            }

            // Check if we should stop reconnecting
            if let Some(max) = config.reconnect.max_attempts
                && attempt >= max
            {
                _ = state_tx.send(ConnectionState::Disconnected);
                _ = diagnostic_tx.send(ConnectionDiagnostic {
                    generation,
                    kind: ConnectionDiagnosticKind::ReconnectExhausted { attempts: attempt },
                });
                break;
            }

            // Update state and wait with exponential backoff
            _ = state_tx.send(ConnectionState::Reconnecting { attempt });

            if let Some(duration) = backoff.next_backoff() {
                tokio::select! {
                    () = sleep(duration) => {}
                    () = wait_for_shutdown(&mut shutdown_rx) => {
                        _ = state_tx.send(ConnectionState::Disconnected);
                        _ = diagnostic_tx.send(ConnectionDiagnostic {
                            generation,
                            kind: ConnectionDiagnosticKind::Shutdown,
                        });
                        break;
                    }
                }
            }
        }

        shutdown.store(true, Ordering::Release);
        _ = closed_tx.send(true);
    }

    /// Handle an active WebSocket connection.
    #[expect(
        clippy::too_many_arguments,
        reason = "the active loop coordinates socket I/O, heartbeat, diagnostics, and shutdown"
    )]
    async fn handle_connection(
        ws_stream: WsStream,
        sender_rx: &mut mpsc::UnboundedReceiver<String>,
        broadcast_tx: &broadcast::Sender<ConnectionEnvelope<M>>,
        diagnostic_tx: &broadcast::Sender<ConnectionDiagnostic>,
        state_rx: watch::Receiver<ConnectionState>,
        shutdown_rx: watch::Receiver<bool>,
        config: Config,
        parser: &P,
        generation: ConnectionGeneration,
    ) -> Result<()> {
        let mut shutdown_rx = shutdown_rx;
        let (mut write, mut read) = ws_stream.split();

        // Channel to notify heartbeat loop when PONG is received
        let (pong_tx, pong_rx) = watch::channel(Instant::now());
        let (ping_tx, mut ping_rx) = mpsc::unbounded_channel();
        let (heartbeat_timeout_tx, mut heartbeat_timeout_rx) = mpsc::unbounded_channel();
        let heartbeat_shutdown_rx = shutdown_rx.clone();
        let heartbeat_config = config.clone();

        let heartbeat_handle = tokio::spawn(async move {
            Self::heartbeat_loop(
                ping_tx,
                heartbeat_timeout_tx,
                state_rx,
                heartbeat_shutdown_rx,
                &heartbeat_config,
                pong_rx,
            )
            .await;
        });

        let result = loop {
            tokio::select! {
                () = wait_for_shutdown(&mut shutdown_rx) => {
                    break Ok(());
                }

                // Handle incoming messages
                Some(msg) = read.next() => {
                    match msg {
                        Ok(Message::Text(text)) if text == "PONG" => {
                            _ = pong_tx.send(Instant::now());
                        }
                        Ok(Message::Text(text)) => {
                            #[cfg(feature = "tracing")]
                            tracing::trace!(%text, "Received WebSocket text message");

                            // Parse messages using the provided parser
                            match parser.parse_with_diagnostics(text.as_bytes()) {
                                Ok(parsed) => {
                                    for diagnostic in parsed.diagnostics {
                                        Self::emit_parser_diagnostic(
                                            diagnostic_tx,
                                            generation,
                                            diagnostic,
                                        );
                                    }
                                    for message in parsed.messages {
                                        #[cfg(feature = "tracing")]
                                        tracing::trace!(?message, "Parsed WebSocket message");
                                        _ = broadcast_tx.send(ConnectionEnvelope {
                                            generation,
                                            message,
                                        });
                                    }
                                }
                                Err(e) => {
                                    let diagnostic = ParserDiagnostic::new(
                                        ParserFailureClassification::MalformedJson,
                                        text.as_bytes(),
                                        None,
                                    );
                                    Self::emit_parser_diagnostic(
                                        diagnostic_tx,
                                        generation,
                                        diagnostic,
                                    );
                                    #[cfg(feature = "tracing")]
                                    tracing::warn!(error = %e, "Failed to parse WebSocket message");
                                    #[cfg(not(feature = "tracing"))]
                                    let _: &_ = &e;
                                }
                            }
                        }
                        Ok(Message::Close(_)) => {
                            _ = diagnostic_tx.send(ConnectionDiagnostic {
                                generation,
                                kind: ConnectionDiagnosticKind::ConnectionClosed,
                            });
                            break Err(Error::with_source(
                                Kind::WebSocket,
                                WsError::ConnectionClosed,
                            ))
                        }
                        Err(e) => {
                            _ = diagnostic_tx.send(ConnectionDiagnostic {
                                generation,
                                kind: ConnectionDiagnosticKind::ConnectionError,
                            });
                            break Err(Error::with_source(
                                Kind::WebSocket,
                                WsError::Connection(e),
                            ));
                        }
                        _ => {
                            // Ignore binary frames and unsolicited PONG replies.
                        }
                    }
                }

                // Handle outgoing messages from subscriptions
                Some(text) = sender_rx.recv() => {
                    if write.send(Message::Text(text.into())).await.is_err() {
                        _ = diagnostic_tx.send(ConnectionDiagnostic {
                            generation,
                            kind: ConnectionDiagnosticKind::WriteFailed,
                        });
                        break Err(Error::with_source(
                            Kind::WebSocket,
                            WsError::ConnectionClosed,
                        ));
                    }
                }

                // Handle PING requests from heartbeat loop
                Some(()) = ping_rx.recv() => {
                    if write.send(Message::Text("PING".into())).await.is_err() {
                        _ = diagnostic_tx.send(ConnectionDiagnostic {
                            generation,
                            kind: ConnectionDiagnosticKind::WriteFailed,
                        });
                        break Err(Error::with_source(
                            Kind::WebSocket,
                            WsError::ConnectionClosed,
                        ));
                    }
                }

                Some(()) = heartbeat_timeout_rx.recv() => {
                    _ = diagnostic_tx.send(ConnectionDiagnostic {
                        generation,
                        kind: ConnectionDiagnosticKind::HeartbeatTimeout {
                            timeout: config.heartbeat_timeout,
                        },
                    });
                    break Err(Error::with_source(Kind::WebSocket, WsError::Timeout));
                }

                // Check if connection is still active
                else => {
                    break Ok(());
                }
            }
        };

        // Cleanup
        heartbeat_handle.abort();
        let _: std::result::Result<(), tokio::task::JoinError> = heartbeat_handle.await;

        result
    }

    /// Heartbeat loop that sends PING messages and monitors PONG responses.
    async fn heartbeat_loop(
        ping_tx: mpsc::UnboundedSender<()>,
        heartbeat_timeout_tx: mpsc::UnboundedSender<()>,
        state_rx: watch::Receiver<ConnectionState>,
        mut shutdown_rx: watch::Receiver<bool>,
        config: &Config,
        mut pong_rx: watch::Receiver<Instant>,
    ) {
        let mut ping_interval = interval(config.heartbeat_interval);

        loop {
            tokio::select! {
                _ = ping_interval.tick() => {}
                () = wait_for_shutdown(&mut shutdown_rx) => break,
            }

            // Check if still connected
            if !state_rx.borrow().is_connected() {
                break;
            }

            // Mark current PONG state as seen before sending PING
            // This prevents changed() from returning immediately due to a stale PONG
            drop(pong_rx.borrow_and_update());

            // Send PING request to message loop
            let ping_sent = Instant::now();
            if ping_tx.send(()).is_err() {
                // Message loop has terminated
                break;
            }

            // Wait for PONG within timeout
            let pong_result = tokio::select! {
                result = timeout(config.heartbeat_timeout, pong_rx.changed()) => result,
                () = wait_for_shutdown(&mut shutdown_rx) => break,
            };

            match pong_result {
                Ok(Ok(())) => {
                    let last_pong = *pong_rx.borrow_and_update();
                    if last_pong < ping_sent {
                        #[cfg(feature = "tracing")]
                        tracing::debug!(
                            "PONG received but older than last PING, connection may be stale"
                        );
                        break;
                    }
                }
                Ok(Err(_)) => {
                    // Channel closed, connection is terminating
                    break;
                }
                Err(_) => {
                    // Timeout waiting for PONG
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        "Heartbeat timeout: no PONG received within {:?}",
                        config.heartbeat_timeout
                    );
                    _ = heartbeat_timeout_tx.send(());
                    break;
                }
            }
        }
    }

    fn emit_parser_diagnostic(
        diagnostic_tx: &broadcast::Sender<ConnectionDiagnostic>,
        generation: ConnectionGeneration,
        diagnostic: ParserDiagnostic,
    ) {
        #[cfg(feature = "tracing")]
        tracing::warn!(
            generation = generation.0,
            classification = ?diagnostic.classification,
            frame_len = diagnostic.frame_len,
            digest = %diagnostic.digest,
            event_type = ?diagnostic.event_type,
            "WebSocket parser diagnostic"
        );
        _ = diagnostic_tx.send(ConnectionDiagnostic {
            generation,
            kind: ConnectionDiagnosticKind::ParserFailure(diagnostic),
        });
    }

    /// Send a subscription request to the WebSocket server.
    pub fn send<R: Serialize>(&self, request: &R) -> Result<()> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(WsError::ConnectionClosed.into());
        }
        let json = serde_json::to_string(request)?;
        self.sender_tx
            .send(json)
            .map_err(|_e| WsError::ConnectionClosed)?;
        Ok(())
    }

    /// Send a subscription request to the WebSocket server.
    pub fn send_authenticated<R: WithCredentials>(
        &self,
        request: &R,
        credentials: &Credentials,
    ) -> Result<()> {
        if self.shutdown.load(Ordering::Acquire) {
            return Err(WsError::ConnectionClosed.into());
        }
        let json = request.as_authenticated(credentials)?;
        self.sender_tx
            .send(json)
            .map_err(|_e| WsError::ConnectionClosed)?;
        Ok(())
    }

    /// Get the current connection state.
    #[must_use]
    pub fn state(&self) -> ConnectionState {
        *self.state_rx.borrow()
    }

    /// Subscribe to incoming messages.
    ///
    /// Each call returns a new independent receiver. Multiple subscribers can
    /// receive messages concurrently without blocking each other.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<ConnectionEnvelope<M>> {
        self.broadcast_tx.subscribe()
    }

    /// Subscribe to connection and parser diagnostics.
    #[must_use]
    pub fn subscribe_diagnostics(&self) -> broadcast::Receiver<ConnectionDiagnostic> {
        self.diagnostic_tx.subscribe()
    }

    /// Get the currently established generation, or zero before the first connection.
    #[must_use]
    pub fn generation(&self) -> ConnectionGeneration {
        *self.generation_rx.borrow()
    }

    /// Subscribe to connection generation changes.
    #[must_use]
    pub fn generation_receiver(&self) -> watch::Receiver<ConnectionGeneration> {
        self.generation_tx.subscribe()
    }

    /// Returns true after explicit shutdown or after the connection task exits permanently.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Explicitly shut down the connection task and reject later writes.
    pub async fn shutdown(&self) -> Result<()> {
        if !self.shutdown.swap(true, Ordering::AcqRel) {
            _ = self.shutdown_tx.send(true);
        }

        let mut closed_rx = self.closed_rx.clone();
        if !*closed_rx.borrow() {
            let _: std::result::Result<(), tokio::time::error::Elapsed> =
                timeout(Duration::from_secs(5), async {
                    while !*closed_rx.borrow() {
                        if closed_rx.changed().await.is_err() {
                            break;
                        }
                    }
                })
                .await;
        }

        if let Some(handle) = self.task.lock().await.take() {
            let _: std::result::Result<(), tokio::task::JoinError> = handle.await;
        }

        Ok(())
    }

    /// Subscribe to connection state changes.
    ///
    /// Returns a receiver that notifies when the connection state changes.
    /// This is useful for detecting reconnections and re-establishing subscriptions.
    #[must_use]
    pub fn state_receiver(&self) -> watch::Receiver<ConnectionState> {
        self.state_tx.subscribe()
    }
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    if *shutdown_rx.borrow() {
        return;
    }

    while shutdown_rx.changed().await.is_ok() {
        if *shutdown_rx.borrow() {
            return;
        }
    }
}
