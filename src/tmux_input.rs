//! Byte-preserving input to an existing tmux pane. Rendering still uses its PTY client.
use std::{
    env, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

pub struct TmuxInput {
    fd: OwnedFd,
    prefix: Vec<u8>,
    queued: Vec<u8>,
    position: usize,
}

impl TmuxInput {
    pub fn from_env() -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let Some(value) = env::var_os("CODEX24H_TMUX_INPUT") else {
            return Ok(None);
        };
        let value = value.to_str().ok_or("invalid tmux input descriptor")?;
        let (fd, pane) = value
            .split_once(':')
            .ok_or("invalid tmux input descriptor")?;
        let fd: i32 = fd.parse()?;
        let pane: u64 = pane
            .strip_prefix('%')
            .ok_or("invalid tmux pane ID")?
            .parse()?;
        if fd < 3 {
            return Err("invalid tmux input descriptor".into());
        }
        // This pipe is inherited only by the wrapper, never by its PTY child.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        crate::terminal::set_nonblocking(fd.as_raw_fd())?;
        Ok(Some(Self {
            fd,
            prefix: format!("send-keys -H -t %{pane}").into_bytes(),
            queued: Vec::new(),
            position: 0,
        }))
    }

    pub fn enqueue(&mut self, bytes: &[u8]) {
        if self.position != 0 {
            self.queued.drain(..self.position);
            self.position = 0;
        }
        const HEX: &[u8] = b"0123456789abcdef";
        // Bound each control command even for large bracketed pastes.
        for chunk in bytes.chunks(512) {
            self.queued.extend_from_slice(&self.prefix);
            for &byte in chunk {
                self.queued.extend_from_slice(&[
                    b' ',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 15) as usize],
                ]);
            }
            self.queued.push(b'\n');
        }
    }

    pub fn pending(&self) -> usize {
        self.queued.len() - self.position
    }

    pub fn pollfd(&self) -> libc::pollfd {
        libc::pollfd {
            fd: if self.pending() == 0 {
                -1
            } else {
                self.fd.as_raw_fd()
            },
            events: if self.pending() == 0 {
                0
            } else {
                libc::POLLOUT
            },
            revents: 0,
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        match crate::terminal::write(self.fd.as_raw_fd(), &self.queued[self.position..]) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => self.position += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
        Ok(())
    }
}
