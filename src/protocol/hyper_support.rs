use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncWrite};



pub struct HyperIo {
    reader: Box<dyn AsyncRead + Unpin + Send + 'static>,
    writer: Box<dyn AsyncWrite + Unpin + Send + 'static>,
}


impl HyperIo {
    pub fn new(io: impl AsyncWrite + AsyncRead + Unpin + Send + 'static) -> Self {
        let (r, w) = tokio::io::split(io);
        HyperIo {
            reader: Box::new(r),
            writer: Box::new(w),
        }
    }
}

impl AsyncWrite for HyperIo {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let s = &mut *self;
        Pin::new(&mut s.writer).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let s = &mut *self;
        Pin::new(&mut s.writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let s = &mut *self;
        Pin::new(&mut s.writer).poll_shutdown(cx)
    }
}

impl AsyncRead for HyperIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let s = &mut *self;
        Pin::new(&mut s.reader).poll_read(cx, buf)
    }
}