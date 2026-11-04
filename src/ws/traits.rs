//! Core traits for generic WebSocket infrastructure.

use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::auth::Credentials;

/// Parser-failure categories that can be surfaced without exposing raw frames.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserFailureClassification {
    /// The frame was not valid JSON for the parser.
    MalformedJson,
    /// The frame had a known, interested event type but did not satisfy its schema.
    InvalidInterestedFrame,
    /// The frame had an unknown event type and was reported as forward-compatible drift.
    UnknownOptionalEvent,
}

/// Bounded parser diagnostic emitted instead of raw WebSocket payloads.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserDiagnostic {
    /// Parser-failure category.
    pub classification: ParserFailureClassification,
    /// Raw frame byte length.
    pub frame_len: usize,
    /// SHA-256 digest of the raw frame, hex encoded.
    pub digest: String,
    /// Wire event type when it was available.
    pub event_type: Option<String>,
}

impl ParserDiagnostic {
    /// Create a bounded diagnostic for a raw frame.
    #[must_use]
    pub fn new(
        classification: ParserFailureClassification,
        bytes: &[u8],
        event_type: Option<String>,
    ) -> Self {
        Self {
            classification,
            frame_len: bytes.len(),
            digest: hex_digest(bytes),
            event_type,
        }
    }
}

/// Parsed messages plus bounded diagnostics observed while parsing a frame.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ParsedMessages<M> {
    /// Successfully parsed messages, preserving source order.
    pub messages: Vec<M>,
    /// Diagnostics for skipped or invalid elements.
    pub diagnostics: Vec<ParserDiagnostic>,
    /// Messages and diagnostics in the exact order observed within the source frame.
    pub items: Vec<ParsedItem<M>>,
}

/// One ordered parser output from a frame.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ParsedItem<M> {
    /// A successfully parsed message.
    Message(M),
    /// A bounded parser diagnostic.
    Diagnostic(ParserDiagnostic),
}

impl<M> ParsedMessages<M> {
    /// Build an outcome with messages and no diagnostics.
    #[must_use]
    pub fn messages(messages: Vec<M>) -> Self {
        Self {
            messages,
            diagnostics: Vec::new(),
            items: Vec::new(),
        }
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);

    for byte in digest {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }

    output
}

/// Message parser trait for converting raw bytes to messages.
///
/// This abstracts the different parsing strategies:
/// - CLOB/WS: Interest-based filtering, peeking at `event_type` before full deserialization
/// - RTDS: Simple parse, no filtering
///
/// # Example
///
/// ```ignore
/// pub struct SimpleParser;
///
/// impl MessageParser<MyMessage> for SimpleParser {
///     fn parse(&self, bytes: &[u8]) -> crate::Result<Vec<MyMessage>> {
///         let msg: MyMessage = serde_json::from_slice(bytes)?;
///         Ok(vec![msg])
///     }
/// }
/// ```
pub trait MessageParser<M: DeserializeOwned>: Send + Sync + 'static {
    /// Parse incoming bytes into messages.
    ///
    /// May return empty vec if messages are filtered out based on interest or other criteria.
    /// Handles both single objects and arrays of messages.
    fn parse(&self, bytes: &[u8]) -> crate::Result<Vec<M>>;

    /// Parse incoming bytes into messages plus bounded diagnostics.
    ///
    /// Parsers that can distinguish forward-compatible unknown events from continuity-breaking
    /// invalid interested frames should override this method. The default preserves the historical
    /// message-only behavior.
    fn parse_with_diagnostics(&self, bytes: &[u8]) -> crate::Result<ParsedMessages<M>> {
        self.parse(bytes).map(ParsedMessages::messages)
    }
}

pub trait WithCredentials: Serialize + Sized {
    fn as_authenticated(&self, credentials: &Credentials) -> Result<String, serde_json::Error> {
        let mut payload_json = serde_json::to_value(self)?;
        let auth = json!({
            "apiKey": credentials.key.to_string(),
            "secret": credentials.secret.expose_secret(),
            "passphrase": credentials.passphrase.expose_secret(),
        });

        if let Value::Object(ref mut obj) = payload_json {
            obj.insert("auth".to_owned(), auth);
        }

        serde_json::to_string(&payload_json)
    }
}
