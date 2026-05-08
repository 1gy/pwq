use crate::error::{Error, Result};
use std::io::{self, ErrorKind, Read, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) trait Backend: Send {
    fn close(&mut self);

    fn wait(&mut self) -> Result<Option<i32>> {
        Ok(None)
    }

    fn set_read_timeout(&mut self, _timeout: Option<Duration>) -> Result<()> {
        Err(Error::Unsupported("set_read_timeout"))
    }
}

struct IoInner {
    reader: Option<Box<dyn Read + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    backend: Box<dyn Backend>,
    buffer: Vec<u8>,
    default_timeout: Option<Duration>,
    current_read_timeout: Option<Duration>,
    /// Distinguishes a definitively-closed `Io` from one mid-`interact()`
    /// (the latter has `reader`/`writer` taken but `closed = false`, so
    /// concurrent ops see `AlreadyTaken` rather than `Closed`).
    closed: bool,
}

pub struct Io {
    inner: Mutex<IoInner>,
}

impl Io {
    pub(crate) fn new(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        backend: Box<dyn Backend>,
    ) -> Self {
        Self {
            inner: Mutex::new(IoInner {
                reader: Some(reader),
                writer: Some(writer),
                backend,
                buffer: Vec::new(),
                default_timeout: None,
                current_read_timeout: None,
                closed: false,
            }),
        }
    }

    pub fn set_timeout(&self, timeout: Option<Duration>) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        apply_read_timeout(&mut g, timeout)?;
        g.default_timeout = timeout;
        Ok(())
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.inner.lock().unwrap().default_timeout
    }

    pub fn send<T: AsRef<[u8]>>(&self, data: T) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        let writer = g.writer.as_mut().ok_or(Error::AlreadyTaken)?;
        writer.write_all(data.as_ref())?;
        writer.flush()?;
        Ok(())
    }

    pub fn send_line<T: AsRef<[u8]>>(&self, data: T) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        let writer = g.writer.as_mut().ok_or(Error::AlreadyTaken)?;
        writer.write_all(data.as_ref())?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    pub fn send_after<P: AsRef<[u8]>, T: AsRef<[u8]>>(&self, prefix: P, data: T) -> Result<()> {
        self.recv_until(prefix)?;
        self.send(data)
    }

    pub fn send_line_after<P: AsRef<[u8]>, T: AsRef<[u8]>>(
        &self,
        prefix: P,
        data: T,
    ) -> Result<()> {
        self.recv_until(prefix)?;
        self.send_line(data)
    }

    pub fn recv(&self, n: usize) -> Result<Vec<u8>> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        let timeout = g.default_timeout;
        recv_inner(&mut g, n, false, timeout)
    }

    pub fn recv_timeout(&self, n: usize, timeout: Duration) -> Result<Vec<u8>> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        recv_inner(&mut g, n, false, Some(timeout))
    }

    pub fn recv_exact(&self, n: usize) -> Result<Vec<u8>> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        let timeout = g.default_timeout;
        recv_inner(&mut g, n, true, timeout)
    }

    pub fn recv_exact_timeout(&self, n: usize, timeout: Duration) -> Result<Vec<u8>> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        recv_inner(&mut g, n, true, Some(timeout))
    }

    pub fn recv_until<P: AsRef<[u8]>>(&self, pattern: P) -> Result<Vec<u8>> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        let timeout = g.default_timeout;
        recv_until_inner(&mut g, pattern.as_ref(), timeout)
    }

    pub fn recv_until_timeout<P: AsRef<[u8]>>(
        &self,
        pattern: P,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Err(Error::Closed);
        }
        recv_until_inner(&mut g, pattern.as_ref(), Some(timeout))
    }

    pub fn recv_line(&self) -> Result<Vec<u8>> {
        self.recv_until(b"\n")
    }

    pub fn recv_line_timeout(&self, timeout: Duration) -> Result<Vec<u8>> {
        self.recv_until_timeout(b"\n", timeout)
    }

    pub fn wait(&self) -> Result<Option<i32>> {
        self.inner.lock().unwrap().backend.wait()
    }

    /// Close streams and backend, dropping buffered data. Subsequent I/O
    /// returns `Error::Closed` (`wait()` still works). Idempotent.
    pub fn close(&self) -> Result<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Ok(());
        }
        g.reader = None;
        g.writer = None;
        g.buffer.clear();
        g.backend.close();
        g.closed = true;
        Ok(())
    }

    /// Bridge stdin/stdout to the backend until it EOFs.
    ///
    /// On return — including via `?` — the `Io` is closed (subsequent
    /// calls return `Error::Closed`). Concurrent `send`/`recv*` while
    /// `interact()` is running return `AlreadyTaken`. The user-stdin
    /// reader is detached, so call `interact()` as the last action before
    /// returning from `main`.
    pub fn interact(&self) -> Result<()> {
        let (mut reader, writer, buffer) = {
            let mut g = self.inner.lock().unwrap();
            if g.closed {
                return Err(Error::Closed);
            }
            let reader = g.reader.take().ok_or(Error::AlreadyTaken)?;
            let writer = g.writer.take().ok_or(Error::AlreadyTaken)?;
            let buffer = std::mem::take(&mut g.buffer);
            // Reads must block until EOF in interactive mode. Failures are
            // ignored defensively: all current backends accept None.
            let _ = apply_read_timeout(&mut g, None);
            (reader, writer, buffer)
        };

        let _guard = InteractGuard { inner: &self.inner };

        if !buffer.is_empty() {
            let mut out = io::stdout();
            out.write_all(&buffer)?;
            out.flush()?;
        }

        let _stdin_thread = std::thread::spawn(move || {
            let mut writer = writer;
            let mut buf = [0u8; 4096];
            let mut user_in = io::stdin();
            loop {
                match user_in.read(&mut buf) {
                    Ok(0) => break,
                    Err(_) => break,
                    Ok(n) => {
                        if writer.write_all(&buf[..n]).is_err() {
                            break;
                        }
                        if writer.flush().is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let mut buf = [0u8; 4096];
        let mut user_out = io::stdout();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Err(_) => break,
                Ok(n) => {
                    if user_out.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = user_out.flush();
                }
            }
        }

        Ok(())
    }
}

/// Closes the `Io` when `interact()` exits, including via `?`.
struct InteractGuard<'a> {
    inner: &'a Mutex<IoInner>,
}

impl Drop for InteractGuard<'_> {
    fn drop(&mut self) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if !g.closed {
            g.backend.close();
            g.closed = true;
        }
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if !g.closed {
            g.reader = None;
            g.writer = None;
            g.backend.close();
            g.closed = true;
        }
    }
}

impl std::fmt::Debug for Io {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Io").finish_non_exhaustive()
    }
}

fn apply_read_timeout(g: &mut IoInner, timeout: Option<Duration>) -> Result<()> {
    if g.current_read_timeout == timeout {
        return Ok(());
    }
    g.backend.set_read_timeout(timeout)?;
    g.current_read_timeout = timeout;
    Ok(())
}

fn recv_inner(
    g: &mut IoInner,
    n: usize,
    exact: bool,
    timeout: Option<Duration>,
) -> Result<Vec<u8>> {
    let deadline = timeout.map(|d| Instant::now() + d);
    while g.buffer.len() < n {
        let remaining = match remaining_timeout(deadline) {
            Some(r) => Some(r),
            None if timeout.is_some() => return Err(Error::Timeout(timeout.unwrap())),
            None => None,
        };
        apply_read_timeout(g, remaining)?;
        let reader = g.reader.as_mut().ok_or(Error::AlreadyTaken)?;
        let mut chunk = [0u8; 4096];
        match reader.read(&mut chunk) {
            Ok(0) => {
                if exact {
                    return Err(Error::Eof);
                }
                if g.buffer.is_empty() {
                    return Err(Error::Eof);
                }
                let out = std::mem::take(&mut g.buffer);
                return Ok(out);
            }
            Ok(read) => g.buffer.extend_from_slice(&chunk[..read]),
            Err(e) if is_timeout(&e) => {
                return Err(Error::Timeout(timeout.unwrap_or(Duration::ZERO)));
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }
    let rest = g.buffer.split_off(n);
    let out = std::mem::replace(&mut g.buffer, rest);
    Ok(out)
}

fn recv_until_inner(g: &mut IoInner, pattern: &[u8], timeout: Option<Duration>) -> Result<Vec<u8>> {
    if pattern.is_empty() {
        return Ok(Vec::new());
    }
    let deadline = timeout.map(|d| Instant::now() + d);
    let mut search_from = 0usize;
    loop {
        if g.buffer.len() >= pattern.len() {
            // Re-scan the last (pattern.len()-1) bytes so patterns straddling
            // a chunk boundary aren't missed.
            let start = search_from.saturating_sub(pattern.len() - 1);
            if let Some(rel) = find_subseq(&g.buffer[start..], pattern) {
                let end = start + rel + pattern.len();
                let rest = g.buffer.split_off(end);
                let out = std::mem::replace(&mut g.buffer, rest);
                return Ok(out);
            }
            search_from = g.buffer.len();
        }
        let remaining = match remaining_timeout(deadline) {
            Some(r) => Some(r),
            None if timeout.is_some() => return Err(Error::Timeout(timeout.unwrap())),
            None => None,
        };
        apply_read_timeout(g, remaining)?;
        let reader = g.reader.as_mut().ok_or(Error::AlreadyTaken)?;
        let mut chunk = [0u8; 4096];
        match reader.read(&mut chunk) {
            Ok(0) => return Err(Error::Eof),
            Ok(read) => g.buffer.extend_from_slice(&chunk[..read]),
            Err(e) if is_timeout(&e) => {
                return Err(Error::Timeout(timeout.unwrap_or(Duration::ZERO)));
            }
            Err(e) => return Err(Error::Io(e)),
        }
    }
}

fn remaining_timeout(deadline: Option<Instant>) -> Option<Duration> {
    let d = deadline?;
    let now = Instant::now();
    if now >= d { None } else { Some(d - now) }
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct NopBackend;
    impl Backend for NopBackend {
        fn close(&mut self) {}
    }

    #[derive(Default)]
    struct TimeoutBackend {
        last: Arc<std::sync::Mutex<Option<Option<Duration>>>>,
        set_count: Arc<AtomicUsize>,
        fail_next: Arc<AtomicBool>,
    }
    impl Backend for TimeoutBackend {
        fn close(&mut self) {}
        fn set_read_timeout(&mut self, t: Option<Duration>) -> Result<()> {
            self.set_count.fetch_add(1, Ordering::Relaxed);
            if self.fail_next.load(Ordering::Relaxed) {
                return Err(Error::Unsupported("toggled"));
            }
            *self.last.lock().unwrap() = Some(t);
            Ok(())
        }
        fn wait(&mut self) -> Result<Option<i32>> {
            Ok(Some(42))
        }
    }

    type TimeoutBackendHandles = (
        Arc<std::sync::Mutex<Option<Option<Duration>>>>,
        Arc<AtomicUsize>,
        Arc<AtomicBool>,
        TimeoutBackend,
    );

    fn timeout_backend() -> TimeoutBackendHandles {
        let backend = TimeoutBackend::default();
        let last = backend.last.clone();
        let count = backend.set_count.clone();
        let fail = backend.fail_next.clone();
        (last, count, fail, backend)
    }

    struct ErrReader(ErrorKind);
    impl Read for ErrReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "test error"))
        }
    }

    fn make_io(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Io {
        Io::new(Box::new(reader), Box::new(writer), Box::new(NopBackend))
    }

    fn io_with_input(data: &[u8]) -> Io {
        make_io(Cursor::new(data.to_vec()), Vec::new())
    }

    struct SharedWriter(Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for SharedWriter {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn shared() -> (Arc<std::sync::Mutex<Vec<u8>>>, SharedWriter) {
        let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
        (buf.clone(), SharedWriter(buf))
    }

    struct ChunkedReader {
        data: Vec<u8>,
        chunk_size: usize,
        pos: usize,
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let want = buf
                .len()
                .min(self.chunk_size)
                .min(self.data.len() - self.pos);
            buf[..want].copy_from_slice(&self.data[self.pos..self.pos + want]);
            self.pos += want;
            Ok(want)
        }
    }

    fn chunked(data: &[u8], chunk_size: usize) -> ChunkedReader {
        ChunkedReader {
            data: data.to_vec(),
            chunk_size,
            pos: 0,
        }
    }

    #[test]
    fn find_subseq_basic() {
        assert_eq!(find_subseq(b"hello world", b"world"), Some(6));
        assert_eq!(find_subseq(b"hello world", b"hello"), Some(0));
        assert_eq!(find_subseq(b"hello world", b"d"), Some(10));
        assert_eq!(find_subseq(b"hello world", b"xyz"), None);
    }

    #[test]
    fn find_subseq_edges() {
        assert_eq!(find_subseq(b"abc", b"abcd"), None);
        assert_eq!(find_subseq(b"", b"x"), None);
        assert_eq!(find_subseq(b"aaaa", b"aa"), Some(0));
        assert_eq!(find_subseq(b"abcabc", b"abc"), Some(0));
    }

    #[test]
    fn recv_returns_partial_on_eof() {
        let io = io_with_input(b"abc");
        assert_eq!(io.recv(10).unwrap(), b"abc");
    }

    #[test]
    fn recv_eof_when_empty() {
        let io = io_with_input(b"");
        assert!(matches!(io.recv(1), Err(Error::Eof)));
    }

    #[test]
    fn recv_zero_returns_empty_without_reading() {
        let io = io_with_input(b"");
        assert_eq!(io.recv(0).unwrap(), b"");
    }

    #[test]
    fn recv_propagates_non_timeout_io_error() {
        let io = make_io(ErrReader(ErrorKind::Other), Vec::<u8>::new());
        match io.recv(1) {
            Err(Error::Io(e)) => assert_eq!(e.kind(), ErrorKind::Other),
            other => panic!("expected Error::Io, got {other:?}"),
        }
    }

    #[test]
    fn recv_until_propagates_non_timeout_io_error() {
        let io = make_io(ErrReader(ErrorKind::ConnectionReset), Vec::<u8>::new());
        match io.recv_until(b"X") {
            Err(Error::Io(e)) => assert_eq!(e.kind(), ErrorKind::ConnectionReset),
            other => panic!("expected Error::Io, got {other:?}"),
        }
    }

    #[test]
    fn recv_exact_returns_n() {
        let io = io_with_input(b"hello world");
        assert_eq!(io.recv_exact(5).unwrap(), b"hello");
    }

    #[test]
    fn recv_exact_short_read_is_eof() {
        let io = io_with_input(b"hi");
        assert!(matches!(io.recv_exact(5), Err(Error::Eof)));
    }

    #[test]
    fn recv_exact_accumulates_across_reads() {
        // chunk_size=3 forces three reads for an 8-byte request.
        let io = make_io(chunked(b"abcdefgh", 3), Vec::new());
        assert_eq!(io.recv_exact(8).unwrap(), b"abcdefgh");
    }

    #[test]
    fn recv_until_empty_pattern_returns_empty() {
        let io = io_with_input(b"anything");
        assert_eq!(io.recv_until(b"").unwrap(), b"");
        assert_eq!(io.recv(8).unwrap(), b"anything");
    }

    #[test]
    fn recv_until_in_initial_buffer() {
        let io = io_with_input(b"hello>world>more");
        assert_eq!(io.recv_until(b">").unwrap(), b"hello>");
    }

    #[test]
    fn recv_until_preserves_remainder() {
        let io = io_with_input(b"hello>world>more");
        assert_eq!(io.recv_until(b">").unwrap(), b"hello>");
        assert_eq!(io.recv_until(b">").unwrap(), b"world>");
        assert_eq!(io.recv(10).unwrap(), b"more");
    }

    #[test]
    fn recv_until_eof_no_pattern() {
        // Buffer reaches >= pattern.len() before EOF: exercises the search-then-EOF path.
        let io = io_with_input(b"no match here");
        assert!(matches!(io.recv_until(b"X"), Err(Error::Eof)));
    }

    #[test]
    fn recv_until_eof_pattern_longer_than_data() {
        // Buffer never reaches pattern.len(): exercises the skip-search branch then EOF.
        let io = io_with_input(b"ab");
        assert!(matches!(io.recv_until(b"longer"), Err(Error::Eof)));
    }

    #[test]
    fn recv_until_pattern_one_byte_chunks() {
        let io = make_io(chunked(b"hello PROMPT> rest", 1), Vec::new());
        assert_eq!(io.recv_until(b"PROMPT>").unwrap(), b"hello PROMPT>");
        assert_eq!(io.recv(10).unwrap(), b" rest");
    }

    #[test]
    fn recv_until_pattern_split_at_chunk_boundary_4() {
        let io = make_io(chunked(b"xyABCD12", 4), Vec::new());
        assert_eq!(io.recv_until(b"ABCD").unwrap(), b"xyABCD");
    }

    #[test]
    fn recv_until_pattern_split_at_chunk_boundary_2() {
        // 2-byte pattern with 2-byte chunks exercises the start = search_from - 1 path.
        let io = make_io(chunked(b"abcXY1", 2), Vec::new());
        assert_eq!(io.recv_until(b"XY").unwrap(), b"abcXY");
    }

    #[test]
    fn recv_until_pattern_at_very_end() {
        let io = io_with_input(b"prefix>");
        assert_eq!(io.recv_until(b">").unwrap(), b"prefix>");
    }

    #[test]
    fn recv_until_after_previous_match() {
        // Second pattern is longer than any single chunk, exercising the
        // skip-search branch across multiple reads after a clean buffer.
        let io = make_io(chunked(b"a>bcDONE", 2), Vec::new());
        assert_eq!(io.recv_until(b">").unwrap(), b"a>");
        assert_eq!(io.recv_until(b"DONE").unwrap(), b"bcDONE");
    }

    #[test]
    fn recv_line_includes_newline() {
        let io = io_with_input(b"first\nsecond\n");
        assert_eq!(io.recv_line().unwrap(), b"first\n");
        assert_eq!(io.recv_line().unwrap(), b"second\n");
    }

    #[test]
    fn recv_line_no_newline_is_eof() {
        let io = io_with_input(b"no newline");
        assert!(matches!(io.recv_line(), Err(Error::Eof)));
    }

    #[test]
    fn send_writes_to_writer() {
        let (buf, w) = shared();
        let io = make_io(Cursor::new(Vec::new()), w);
        io.send(b"hello").unwrap();
        io.send_line(b"world").unwrap();
        assert_eq!(&*buf.lock().unwrap(), b"helloworld\n");
    }

    #[test]
    fn send_line_after_consumes_prefix_then_sends() {
        let (buf, w) = shared();
        let io = make_io(Cursor::new(b"Size: ".to_vec()), w);
        io.send_line_after("Size: ", b"42").unwrap();
        assert_eq!(&*buf.lock().unwrap(), b"42\n");
    }

    #[test]
    fn send_after_does_not_append_newline() {
        let (buf, w) = shared();
        let io = make_io(Cursor::new(b"go".to_vec()), w);
        io.send_after("go", b"raw").unwrap();
        assert_eq!(&*buf.lock().unwrap(), b"raw");
    }

    #[test]
    fn close_then_send_returns_closed() {
        let io = io_with_input(b"");
        io.close().unwrap();
        assert!(matches!(io.send(b"x"), Err(Error::Closed)));
    }

    #[test]
    fn close_then_recv_returns_closed() {
        let io = io_with_input(b"");
        io.close().unwrap();
        assert!(matches!(io.recv(1), Err(Error::Closed)));
    }

    #[test]
    fn close_drops_pending_buffer() {
        let io = io_with_input(b"hello world");
        let _ = io.recv_until(b" ").unwrap(); // leaves "world" in the internal buffer
        io.close().unwrap();
        assert!(matches!(io.recv(5), Err(Error::Closed)));
        assert!(matches!(io.recv_until(b"d"), Err(Error::Closed)));
    }

    #[test]
    fn close_then_interact_returns_closed() {
        let io = io_with_input(b"");
        io.close().unwrap();
        assert!(matches!(io.interact(), Err(Error::Closed)));
    }

    #[test]
    fn close_is_idempotent() {
        let io = io_with_input(b"");
        io.close().unwrap();
        io.close().unwrap();
    }

    #[test]
    fn close_then_set_timeout_returns_closed() {
        let io = io_with_input(b"");
        io.close().unwrap();
        assert!(matches!(io.set_timeout(None), Err(Error::Closed)));
    }

    #[test]
    fn interact_marks_io_closed_on_normal_return() {
        // Empty reader → main loop exits at once; detached stdin thread is
        // harmless in the test environment.
        let io = io_with_input(b"");
        io.interact().unwrap();
        assert!(matches!(io.recv(1), Err(Error::Closed)));
        assert!(matches!(io.send(b"x"), Err(Error::Closed)));
        assert!(matches!(io.interact(), Err(Error::Closed)));
    }

    #[test]
    fn set_timeout_none_succeeds_on_unsupporting_backend() {
        // NopBackend rejects set_read_timeout, but apply_read_timeout
        // short-circuits when the requested value already matches current.
        let io = io_with_input(b"");
        assert_eq!(io.timeout(), None);
        io.set_timeout(None).unwrap();
        assert_eq!(io.timeout(), None);
    }

    #[test]
    fn set_timeout_failure_does_not_mutate_state() {
        let io = io_with_input(b"");
        assert_eq!(io.timeout(), None);
        let err = io.set_timeout(Some(Duration::from_secs(1))).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
        assert_eq!(io.timeout(), None);
    }

    #[test]
    fn set_timeout_success_updates_state_and_propagates_to_backend() {
        let (last, _count, _fail, backend) = timeout_backend();
        let io = Io::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Box::new(Vec::<u8>::new()),
            Box::new(backend),
        );
        let d = Duration::from_secs(5);
        io.set_timeout(Some(d)).unwrap();
        assert_eq!(io.timeout(), Some(d));
        assert_eq!(*last.lock().unwrap(), Some(Some(d)));
    }

    #[test]
    fn set_timeout_same_value_short_circuits_backend() {
        let (_last, count, _fail, backend) = timeout_backend();
        let io = Io::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Box::new(Vec::<u8>::new()),
            Box::new(backend),
        );
        let d = Duration::from_secs(5);
        io.set_timeout(Some(d)).unwrap();
        io.set_timeout(Some(d)).unwrap();
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn set_timeout_failure_after_success_preserves_previous_value() {
        let (_last, _count, fail, backend) = timeout_backend();
        let io = Io::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Box::new(Vec::<u8>::new()),
            Box::new(backend),
        );
        let d1 = Duration::from_secs(5);
        let d2 = Duration::from_secs(10);
        io.set_timeout(Some(d1)).unwrap();
        fail.store(true, Ordering::Relaxed);
        let err = io.set_timeout(Some(d2)).unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
        assert_eq!(io.timeout(), Some(d1));
    }

    #[test]
    fn recv_until_timeout_propagates_to_backend() {
        // Empty reader: EOF fires right after the timeout is applied, so
        // we observe propagation without depending on timeout firing.
        let (last, _count, _fail, backend) = timeout_backend();
        let io = Io::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Box::new(Vec::<u8>::new()),
            Box::new(backend),
        );
        let _ = io.recv_until_timeout(b"X", Duration::from_secs(1));
        // Exact value differs by sub-microsecond elapsed time, so check shape only.
        assert!(matches!(*last.lock().unwrap(), Some(Some(_))));
    }

    #[test]
    fn recv_exact_timeout_propagates_to_backend() {
        let (last, _count, _fail, backend) = timeout_backend();
        let io = Io::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Box::new(Vec::<u8>::new()),
            Box::new(backend),
        );
        let _ = io.recv_exact_timeout(8, Duration::from_secs(1));
        assert!(matches!(*last.lock().unwrap(), Some(Some(_))));
    }

    #[test]
    fn wait_default_returns_none() {
        let io = io_with_input(b"");
        assert_eq!(io.wait().unwrap(), None);
    }

    #[test]
    fn wait_returns_backend_value() {
        let (_last, _count, _fail, backend) = timeout_backend();
        let io = Io::new(
            Box::new(Cursor::new(Vec::<u8>::new())),
            Box::new(Vec::<u8>::new()),
            Box::new(backend),
        );
        assert_eq!(io.wait().unwrap(), Some(42));
    }
}
