use std::fmt::Debug;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

use serde_json;

use crate::errors::{DeserializationError, ServerError};
use crate::protocol_message::Sendable;
use crate::requests::Request;
use crate::{
  events::Event, protocol_message::DAPMessage, responses::Response,
  reverse_requests::ReverseRequest,
};

#[derive(Debug)]
enum ServerState {
  /// Expecting a header
  Header,
  /// Expecting a separator between header and content, i.e. "\r\n"
  Sep,
  /// Expecting content
  Content,
}

/// Ties together an Adapter and a Client.
///
/// The `Server` is responsible for reading the incoming bytestream and constructing deserialized
/// requests from it; calling the `accept` function of the `Adapter` and passing the response
/// to the client.
pub struct Server<R: Read, W: Write> {
  input_buffer: BufReader<R>,
  output_buffer: BufWriter<W>,
  seq: i64,
}

impl<R: Read, W: Write> Server<R, W> {
  /// Construct a new Server and take ownership of the adapter and client.
  pub fn new(input: BufReader<R>, output: BufWriter<W>) -> Self {
    Self {
      input_buffer: input,
      output_buffer: output,
      seq: 0,
    }
  }

  fn next_seq(&mut self) -> i64 {
    self.seq += 1;
    self.seq
  }

  /// Run the server.
  ///
  /// This will start reading the `input` buffer that is passed to it and will try to interpert
  /// the incoming bytes according to the DAP protocol.
  pub fn poll_request(&mut self) -> Result<Option<Request>, ServerError> {
    let mut state = ServerState::Header;
    let mut buffer = String::new();
    let mut content_length: usize = 0;

    loop {
      match self.input_buffer.read_line(&mut buffer) {
        Ok(mut read_size) => {
          if read_size == 0 {
            break Ok(None);
          }
          match state {
            ServerState::Header => {
              let parts: Vec<&str> = buffer.trim_end().split(':').collect();
              if parts.len() == 2 {
                match parts[0] {
                  "Content-Length" => {
                    content_length = match parts[1].trim().parse() {
                      Ok(val) => val,
                      Err(_) => return Err(ServerError::HeaderParseError { line: buffer }),
                    };
                    buffer.clear();
                    buffer.reserve(content_length);
                    state = ServerState::Sep;
                  }
                  other => {
                    return Err(ServerError::UnknownHeader {
                      header: other.to_string(),
                    })
                  }
                }
              } else {
                return Err(ServerError::HeaderParseError { line: buffer });
              }
            }
            ServerState::Sep => {
              if buffer == "\r\n" {
                state = ServerState::Content;
                buffer.clear();
              }
            }
            ServerState::Content => {
              while read_size < content_length {
                read_size += self.input_buffer.read_line(&mut buffer).unwrap();
              }
              let request: Request = match serde_json::from_str(&buffer) {
                Ok(val) => val,
                Err(e) => return Err(ServerError::ParseError(DeserializationError::SerdeError(e))),
              };
              return Ok(Some(request));
            }
          }
        }
        Err(_) => return Err(ServerError::IoError),
      }
    }
  }

  pub fn send(&mut self, message: DAPMessage) -> Result<(), ServerError> {
    let resp_json = serde_json::to_string(&message).map_err(ServerError::SerializationError)?;
    write!(
      self.output_buffer,
      "Content-Length: {}\r\n\r\n",
      resp_json.len()
    )
    .unwrap();

    write!(self.output_buffer, "{}\r\n", resp_json).unwrap();
    self.output_buffer.flush().unwrap();
    Ok(())
  }

  pub fn respond(&mut self, response: Response) -> Result<(), ServerError> {
    let message = DAPMessage {
      seq: self.next_seq(),
      message: Sendable::Response(response),
    };
    self.send(message)
  }

  pub fn send_event(&mut self, event: Event) -> Result<(), ServerError> {
    let message = DAPMessage {
      seq: self.next_seq(),
      message: Sendable::Event(event),
    };
    self.send(message)
  }

  pub fn send_reverse_request(&mut self, request: ReverseRequest) -> Result<(), ServerError> {
    let message = DAPMessage {
      seq: self.next_seq(),
      message: Sendable::ReverseRequest(request),
    };
    self.send(message)
  }
}
