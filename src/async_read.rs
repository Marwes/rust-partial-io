/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! This module contains an `AsyncRead` wrapper that breaks its inputs up
//! according to a provided iterator.
//!
//! This is separate from `PartialWrite` because on `WouldBlock` errors, it
//! causes `futures` to try writing or flushing again.

use std::cmp;
use std::fmt;
use std::io::{self, Read, Write};

use futures::{task, Poll};
use tokio_io::{AsyncRead, AsyncWrite};

use crate::{make_ops, PartialOp};

/// A wrapper that breaks inner `AsyncRead` instances up according to the
/// provided iterator.
///
/// Available with the `tokio` feature.
///
/// # Examples
///
/// ```rust
/// use std::io::{self, Cursor};
///
/// fn main() {
///     use tokio_core::reactor::Core;
///     use tokio_io::io::read as tokio_read;
///
///     use partial_io::{PartialAsyncRead, PartialOp};
///
///     let reader = Cursor::new(vec![1, 2, 3, 4]);
///     let iter = vec![PartialOp::Err(io::ErrorKind::WouldBlock), PartialOp::Limited(2)];
///     let partial_reader = PartialAsyncRead::new(reader, iter);
///     let out = vec![0; 256];
///
///     let mut core = Core::new().unwrap();
///
///     // This future will skip over the WouldBlock and return however much was
///     // successfully read the first time a read succeeded.
///     let read_fut = tokio_read(partial_reader, out);
///
///     let (_partial_reader, out, size) = core.run(read_fut).unwrap();
///
///     assert_eq!(size, 2);
///     assert_eq!(&out[..3], &[1, 2, 0]);
/// }
/// ```
pub struct PartialAsyncRead<R> {
    inner: R,
    ops: Box<dyn Iterator<Item = PartialOp> + Send>,
}

impl<R> PartialAsyncRead<R>
where
    R: AsyncRead,
{
    /// Creates a new `PartialAsyncRead` wrapper over the reader with the specified `PartialOp`s.
    pub fn new<I>(inner: R, iter: I) -> Self
    where
        I: IntoIterator<Item = PartialOp> + 'static,
        I::IntoIter: Send,
    {
        PartialAsyncRead {
            inner,
            ops: make_ops(iter),
        }
    }

    /// Sets the `PartialOp`s for this reader.
    pub fn set_ops<I>(&mut self, iter: I) -> &mut Self
    where
        I: IntoIterator<Item = PartialOp> + 'static,
        I::IntoIter: Send,
    {
        self.ops = make_ops(iter);
        self
    }

    /// Acquires a reference to the underlying reader.
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Acquires a mutable reference to the underlying reader.
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Consumes this wrapper, returning the underlying reader.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R> Read for PartialAsyncRead<R>
where
    R: AsyncRead,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.ops.next() {
            Some(PartialOp::Limited(n)) => {
                let len = cmp::min(n, buf.len());
                self.inner.read(&mut buf[..len])
            }
            Some(PartialOp::Err(err)) => {
                if err == io::ErrorKind::WouldBlock {
                    // Make sure this task is rechecked.
                    task::park().unpark();
                }
                Err(io::Error::new(
                    err,
                    "error during read, generated by partial-io",
                ))
            }
            Some(PartialOp::Unlimited) | None => self.inner.read(buf),
        }
    }
}

impl<R> AsyncRead for PartialAsyncRead<R> where R: AsyncRead {}

// Forwarding impls to support duplex structs.
impl<R> Write for PartialAsyncRead<R>
where
    R: AsyncRead + Write,
{
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<R> AsyncWrite for PartialAsyncRead<R>
where
    R: AsyncRead + AsyncWrite,
{
    #[inline]
    fn shutdown(&mut self) -> Poll<(), io::Error> {
        self.inner.shutdown()
    }
}

impl<R> fmt::Debug for PartialAsyncRead<R>
where
    R: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PartialAsyncRead")
            .field("inner", &self.inner)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::File;

    use crate::tests::assert_send;

    #[test]
    fn test_sendable() {
        assert_send::<PartialAsyncRead<File>>();
    }
}
