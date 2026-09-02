use std::fmt;

use crate::clob::ws::types::response::WsMessage;
use crate::ws::{ConnectionDiagnosticKind, ConnectionGeneration, ParserDiagnostic};

/// Consumer-visible event from the ordered public market stream.
#[non_exhaustive]
#[expect(
    clippy::large_enum_variant,
    reason = "market messages stay by-value so stream consumers can match the existing WsMessage API"
)]
#[derive(Debug, Clone)]
pub enum MarketStreamEvent {
    /// A market message received on a concrete connection generation.
    Message {
        /// Immutable connection generation for this message.
        generation: ConnectionGeneration,
        /// Parsed market message.
        message: WsMessage,
    },
    /// A continuity or lifecycle boundary that consumers must observe.
    Continuity {
        /// Immutable connection generation affected by this boundary.
        generation: ConnectionGeneration,
        /// Typed boundary reason.
        reason: MarketStreamContinuity,
    },
    /// Terminal stream closure evidence.
    Terminal {
        /// Immutable connection generation that closed or zero before any connection.
        generation: ConnectionGeneration,
        /// Typed terminal reason.
        reason: MarketStreamTerminal,
    },
}

impl MarketStreamEvent {
    /// Return the generation attached to this event.
    #[must_use]
    pub const fn generation(&self) -> ConnectionGeneration {
        match self {
            Self::Message { generation, .. }
            | Self::Continuity { generation, .. }
            | Self::Terminal { generation, .. } => *generation,
        }
    }
}

/// Non-terminal continuity and lifecycle reasons for a market stream.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketStreamContinuity {
    /// A connection generation was established or became visible to the stream.
    Connected,
    /// The connection closed before a later generation may be established.
    Disconnected,
    /// The consumer lagged the broadcast buffer.
    Lagged {
        /// Number of missed messages reported by Tokio broadcast.
        missed: u64,
    },
    /// A parser issue was surfaced without exposing the full raw payload.
    ParserDiagnostic(ParserDiagnostic),
    /// The heartbeat path timed out while waiting for PONG.
    HeartbeatTimeout,
    /// A single connection attempt timed out.
    ConnectTimeout,
    /// An outbound write failed and delivery must not be assumed.
    WriteFailed,
}

/// Terminal reasons for explicit close or exhausted connection policy.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketStreamTerminal {
    /// The caller explicitly shut the SDK connection down.
    Shutdown,
    /// Reconnect policy reached its configured attempt limit.
    ReconnectExhausted {
        /// Number of attempts consumed by the policy.
        attempts: u32,
    },
    /// The underlying diagnostic channel was closed.
    ChannelClosed,
}

impl MarketStreamContinuity {
    /// Convert a non-terminal low-level connection diagnostic into a market stream reason.
    #[must_use]
    pub fn from_connection_diagnostic(kind: ConnectionDiagnosticKind) -> Option<Self> {
        match kind {
            ConnectionDiagnosticKind::Connected => Some(Self::Connected),
            ConnectionDiagnosticKind::ConnectionClosed
            | ConnectionDiagnosticKind::ConnectionError => Some(Self::Disconnected),
            ConnectionDiagnosticKind::ParserFailure(diagnostic) => {
                Some(Self::ParserDiagnostic(diagnostic))
            }
            ConnectionDiagnosticKind::HeartbeatTimeout { .. } => Some(Self::HeartbeatTimeout),
            ConnectionDiagnosticKind::ConnectTimeout { .. } => Some(Self::ConnectTimeout),
            ConnectionDiagnosticKind::WriteFailed => Some(Self::WriteFailed),
            ConnectionDiagnosticKind::ReconnectExhausted { .. }
            | ConnectionDiagnosticKind::Shutdown => None,
        }
    }
}

impl fmt::Display for MarketStreamTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shutdown => f.write_str("shutdown"),
            Self::ReconnectExhausted { attempts } => {
                write!(f, "reconnect exhausted after {attempts} attempts")
            }
            Self::ChannelClosed => f.write_str("channel closed"),
        }
    }
}
