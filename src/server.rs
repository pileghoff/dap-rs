use std::fmt::Debug;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

use serde_json;

use crate::{
  base_message::{BaseMessage, Sendable},
  errors::{DeserializationError, ServerError},
  requests::Request,
};

#[derive(Debug)]
enum ServerState {
  /// Expecting a header
  Header,
  /// Expecting content
  Content,
}

/// Handles message encoding and decoding of messages.
///
/// The `Server` is responsible for reading the incoming bytestream and constructing deserialized
/// requests from it, as well as constructing and serializing outgoing messages.
pub struct Server<R: Read, W: Write> {
  input_buffer: BufReader<R>,
  output_buffer: BufWriter<W>,
  sequence_number: i64,
}

impl<R: Read, W: Write> Server<R, W> {
  /// Construct a new Server using the given input and output streams.
  pub fn new(input: BufReader<R>, output: BufWriter<W>) -> Self {
    Self {
      input_buffer: input,
      output_buffer: output,
      sequence_number: 0,
    }
  }

  /// Wait for a request from the development tool
  ///
  /// This will start reading the `input` buffer that is passed to it and will try to interpret
  /// the incoming bytes according to the DAP protocol.
  pub fn poll_request(&mut self) -> Result<Option<Request>, ServerError> {
    let mut state = ServerState::Header;
    let mut buffer = String::new();
    let mut content_length: usize = 0;

    loop {
      match self.input_buffer.read_line(&mut buffer) {
        Ok(read_size) => {
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
                    state = ServerState::Content;
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
            ServerState::Content => {
              buffer.clear();
              let mut content = vec![0; content_length];
              if self
                .input_buffer
                .read_exact(content.as_mut_slice())
                .is_err()
              {
                return Err(ServerError::IoError);
              }
              let content = std::str::from_utf8(content.as_slice()).unwrap();
              let request: Request = match serde_json::from_str(content) {
                Ok(val) => val,
                Err(e) => {
                  return Err(ServerError::ParseError(DeserializationError::SerdeError(e)));
                }
              };
              return Ok(Some(request));
            }
          }
        }
        Err(_) => return Err(ServerError::IoError),
      }
    }
  }

  pub fn send<T: Into<Sendable>>(&mut self, body: T) -> Result<(), ServerError> {
    self.sequence_number += 1;

    let message = BaseMessage {
      seq: self.sequence_number,
      message: body.into(),
    };

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
}

#[cfg(test)]
mod tests {

  use std::io::Cursor;

  use super::*;
  use crate::requests::Command;

  #[test]
  fn test_server_init_request() {
    let mut server_in = Cursor::new("Content-Length: 155\r\n\r\n{\"seq\": 152,\"type\": \"request\",\"command\": \"initialize\",\"arguments\": {\"adapterID\": \"0001e357-72c7-4f03-ae8f-c5b54bd8dabf\", \"clientName\": \"Some Cool Editor\"}}".as_bytes().to_vec());
    let server_out = Vec::new();
    let mut server = Server::new(BufReader::new(&mut server_in), BufWriter::new(server_out));

    let req = server.poll_request().unwrap().unwrap();
    assert_eq!(req.seq, 152);
    assert!(matches!(req.command, Command::Initialize { .. }));
  }
}
