use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use md_trace::{PROTOCOL_VERSION, TraceRecord, WireError, decode, read_payload};

#[derive(Debug, thiserror::Error)]
pub enum TraceClientError {
    #[error("cannot connect to {path}: {source}; is magic-driver running?")]
    Connect { path: String, source: io::Error },
    #[error("{0}")]
    Wire(#[from] WireError),
    #[error("the daemon did not start with a hello record")]
    NoHello,
    #[error("the daemon speaks trace protocol {0}, this client speaks {PROTOCOL_VERSION}")]
    Protocol(u32),
    #[error("cannot write the output: {0}")]
    Output(io::Error),
}

pub fn run<W: Write>(path: &Path, out: W) -> Result<u64, TraceClientError> {
    let stream = UnixStream::connect(path).map_err(|source| TraceClientError::Connect {
        path: path.display().to_string(),
        source,
    })?;
    print_stream(stream, out)
}

pub fn print_stream<R: io::Read, W: Write>(
    mut stream: R,
    mut out: W,
) -> Result<u64, TraceClientError> {
    let mut records = 0;
    while let Some(payload) = read_payload(&mut stream)? {
        if records == 0 {
            match decode(&payload)? {
                TraceRecord::Hello(hello) if hello.protocol_version == PROTOCOL_VERSION => {}
                TraceRecord::Hello(hello) => {
                    return Err(TraceClientError::Protocol(hello.protocol_version));
                }
                _ => return Err(TraceClientError::NoHello),
            }
        }
        out.write_all(&payload).map_err(TraceClientError::Output)?;
        out.write_all(b"\n").map_err(TraceClientError::Output)?;
        out.flush().map_err(TraceClientError::Output)?;
        records += 1;
    }
    Ok(records)
}
