use crate::error::Result;
use crate::io::{Backend, Io};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;

struct RemoteBackend {
    stream: TcpStream,
}

impl Backend for RemoteBackend {
    fn close(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.stream.set_read_timeout(timeout)?;
        Ok(())
    }
}

pub fn remote<A: ToSocketAddrs>(addr: A) -> Result<Io> {
    let stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    let reader = stream.try_clone()?;
    let writer = stream.try_clone()?;
    Ok(Io::new(
        Box::new(reader),
        Box::new(writer),
        Box::new(RemoteBackend { stream }),
    ))
}
