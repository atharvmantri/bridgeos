use bridge_core::{BridgeError, Result};
use bridge_protocol::{decode_payload, encode_frame, Frame, MAGIC, MAX_FRAME_LENGTH};
use std::io::ErrorKind;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Bounded length-delimited framed stream over an asynchronous transport.
#[derive(Debug)]
pub struct FramedStream<T> {
    inner: T,
}

impl<T> FramedStream<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Receives the next complete `Frame` from the stream.
    ///
    /// Returns:
    /// - `Ok(Some(frame))` on successful receipt of a full frame.
    /// - `Ok(None)` if the remote peer closed the stream cleanly at a frame boundary.
    /// - `Err(BridgeError)` on wire corruption, invalid magic, size violation, or unexpected EOF.
    pub async fn recv_frame(&mut self) -> Result<Option<Frame>> {
        let mut header = [0u8; 8];
        match self.inner.read_exact(&mut header).await {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => {
                // Clean EOF at frame boundary
                return Ok(None);
            }
            Err(e) => return Err(BridgeError::Internal(format!("Transport read error: {e}"))),
        }

        // Verify magic prefix
        if header[0..4] != MAGIC {
            return Err(BridgeError::InvalidMagic);
        }

        let length = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
        if length > MAX_FRAME_LENGTH {
            return Err(BridgeError::FrameTooLarge {
                length,
                max: MAX_FRAME_LENGTH,
            });
        }

        // Allocate bounded buffer and read exact payload
        let mut payload = vec![0u8; length];
        self.inner
            .read_exact(&mut payload)
            .await
            .map_err(|e| BridgeError::Framing(format!("Failed to read frame payload: {e}")))?;

        let frame = decode_payload(&payload)?;
        Ok(Some(frame))
    }

    /// Encodes and transmits a `Frame` across the stream, ensuring buffers are flushed.
    pub async fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let encoded = encode_frame(frame)?;
        self.inner
            .write_all(&encoded)
            .await
            .map_err(|e| BridgeError::Internal(format!("Transport write error: {e}")))?;
        self.inner
            .flush()
            .await
            .map_err(|e| BridgeError::Internal(format!("Transport flush error: {e}")))?;
        Ok(())
    }

    /// Destructures this wrapper to access the underlying I/O object.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_protocol::{ControlFrame, DataFrame};
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_framed_stream_ping_pong() {
        let (client_io, server_io) = duplex(64 * 1024);
        let mut client = FramedStream::new(client_io);
        let mut server = FramedStream::new(server_io);

        // Client sends Ping
        let ping = Frame::Control(ControlFrame::Ping { nonce: 42 });
        client.send_frame(&ping).await.expect("Failed to send ping");

        // Server receives Ping and replies with Pong
        let received = server
            .recv_frame()
            .await
            .expect("Server recv error")
            .expect("Unexpected EOF");
        assert_eq!(received, ping);

        let pong = Frame::Control(ControlFrame::Pong { nonce: 42 });
        server.send_frame(&pong).await.expect("Failed to send pong");

        // Client receives Pong
        let client_recv = client
            .recv_frame()
            .await
            .expect("Client recv error")
            .expect("Unexpected EOF");
        assert_eq!(client_recv, pong);
    }

    #[tokio::test]
    async fn test_data_channel_frame_transfer() {
        let (client_io, server_io) = duplex(64 * 1024);
        let mut client = FramedStream::new(client_io);
        let mut server = FramedStream::new(server_io);

        let test_data = b"BridgeOS Clipboard Synchronization Payload".to_vec();
        let frame = Frame::Data(DataFrame::new(
            DataFrame::CHANNEL_CLIPBOARD,
            test_data.clone(),
        ));

        client.send_frame(&frame).await.expect("Send failed");
        let recv = server
            .recv_frame()
            .await
            .expect("Recv failed")
            .expect("EOF");

        match recv {
            Frame::Data(data) => {
                assert_eq!(data.channel, DataFrame::CHANNEL_CLIPBOARD);
                assert_eq!(data.payload, test_data);
            }
            other => panic!("Expected DataFrame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_clean_eof_handling() {
        let (client_io, server_io) = duplex(64 * 1024);
        let client = FramedStream::new(client_io);
        let mut server = FramedStream::new(server_io);

        drop(client);
        let res = server.recv_frame().await.expect("Recv error");
        assert!(res.is_none(), "Expected clean EOF (None)");
    }
}
