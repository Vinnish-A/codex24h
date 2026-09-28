//! Unix terminal ownership and capability discovery.
use crate::core::Size;
use alacritty_terminal::vte::ansi::Rgb;
use std::{
    io,
    mem::MaybeUninit,
    time::{Duration, Instant},
};
pub struct Terminal {
    original: libc::termios,
    input_flags: i32,
    output_flags: i32,
    entered: bool,
    pub kitty: bool,
}
pub struct Capabilities {
    pub kitty: bool,
    pub foreground: Rgb,
    pub background: Rgb,
    pub pending: Vec<u8>,
}
pub fn size() -> io::Result<Size> {
    let mut value = MaybeUninit::<libc::winsize>::zeroed();
    if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, value.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let value = unsafe { value.assume_init() };
    if value.ws_row < 2 || value.ws_col < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal must be at least 2 rows and 2 columns",
        ));
    }
    Ok(Size {
        rows: value.ws_row as usize - 1,
        cols: value.ws_col as usize,
    })
}
pub fn set_nonblocking(fd: i32) -> io::Result<i32> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(flags)
}
pub fn read(fd: i32, buf: &mut [u8]) -> io::Result<usize> {
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}
pub fn write(fd: i32, buf: &[u8]) -> io::Result<usize> {
    let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}
pub fn is_tty(fd: i32) -> bool {
    (unsafe { libc::isatty(fd) }) == 1
}
pub fn poll(fds: &mut [libc::pollfd], millis: i32) -> io::Result<()> {
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) };
    if rc < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn write_deadline(bytes: &[u8], duration: Duration) -> io::Result<()> {
    let end = Instant::now() + duration;
    let mut pos = 0;
    while pos < bytes.len() {
        match write(1, &bytes[pos..]) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(n) => pos += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            Err(e) => return Err(e),
        }
        if pos < bytes.len() {
            if Instant::now() >= end {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }
            poll(
                &mut [libc::pollfd {
                    fd: 1,
                    events: libc::POLLOUT,
                    revents: 0,
                }],
                10,
            )?;
        }
    }
    Ok(())
}
impl Terminal {
    pub fn raw() -> io::Result<Self> {
        let mut original = MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(0, original.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        unsafe { libc::cfmakeraw(&mut raw) };
        let input_flags = unsafe { libc::fcntl(0, libc::F_GETFL) };
        let output_flags = unsafe { libc::fcntl(1, libc::F_GETFL) };
        if input_flags < 0 || output_flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let guard = Self {
            original,
            input_flags,
            output_flags,
            entered: false,
            kitty: false,
        };
        set_nonblocking(0)?;
        set_nonblocking(1)?;
        Ok(guard)
    }
    pub fn discover(&self) -> io::Result<Capabilities> {
        write_deadline(
            b"\x1b[?u\x1b]10;?\x1b\\\x1b]11;?\x1b\\",
            Duration::from_millis(250),
        )?;
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut bytes = Vec::new();
        let mut buf = [0; 4096];
        while Instant::now() < deadline {
            poll(
                &mut [libc::pollfd {
                    fd: 0,
                    events: libc::POLLIN,
                    revents: 0,
                }],
                10,
            )?;
            match read(0, &mut buf) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(parse_capabilities(&bytes))
    }
    pub fn enter(&mut self, kitty: bool) -> io::Result<()> {
        self.entered = true;
        self.kitty = kitty;
        write_deadline(
            b"\x1b[22;0t\x1b[?1049h\x1b[?25l\x1b[?1002h\x1b[?1006h",
            Duration::from_secs(2),
        )?;
        if kitty {
            write_deadline(b"\x1b[>0u", Duration::from_secs(2))?;
        }
        Ok(())
    }
    /// Leave the alternate screen and restore the invoking shell's TTY before a job stop.
    pub fn suspend(&mut self) -> io::Result<()> {
        let mut error = None;
        if self.entered {
            let mut cleanup=b"\x1b[?2026l\x1b]8;;\x1b\\\x1b[0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[?1l\x1b>\x1b[?7h\x1b[0 q\x1b[?25h".to_vec();
            if self.kitty {
                cleanup.extend_from_slice(b"\x1b[<u");
            }
            cleanup.extend_from_slice(b"\x1b[?1049l\x1b[23;0t");
            if let Err(e) = write_deadline(&cleanup, Duration::from_millis(300)) {
                error = Some(e);
            }
            self.entered = false;
        }
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.original) } < 0 && error.is_none() {
            error = Some(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(0, libc::F_SETFL, self.input_flags) } < 0 && error.is_none() {
            error = Some(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(1, libc::F_SETFL, self.output_flags) } < 0 && error.is_none() {
            error = Some(io::Error::last_os_error());
        }
        error.map_or(Ok(()), Err)
    }
    /// Re-enter the wrapper after SIGCONT, preserving the original shell state for Drop.
    pub fn resume(&mut self) -> io::Result<()> {
        if self.entered {
            return Ok(());
        }
        let mut raw = self.original;
        unsafe { libc::cfmakeraw(&mut raw) };
        let result = (|| {
            if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } < 0 {
                return Err(io::Error::last_os_error());
            }
            set_nonblocking(0)?;
            set_nonblocking(1)?;
            self.enter(self.kitty)
        })();
        if result.is_err() {
            let _ = self.suspend();
        }
        result
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}
fn parse_capabilities(bytes: &[u8]) -> Capabilities {
    let mut caps = Capabilities {
        kitty: false,
        foreground: Rgb {
            r: 229,
            g: 229,
            b: 229,
        },
        background: Rgb { r: 0, g: 0, b: 0 },
        pending: Vec::new(),
    };
    let mut pos = 0;
    while pos < bytes.len() {
        let tail = &bytes[pos..];
        if tail.starts_with(b"\x1b[?") {
            if let Some(end) = tail[3..]
                .iter()
                .position(|b| (0x40..=0x7e).contains(b))
                .map(|n| n + 3)
            {
                if tail[end] == b'u'
                    && !tail[3..end].is_empty()
                    && tail[3..end].iter().all(u8::is_ascii_digit)
                {
                    caps.kitty = true;
                    pos += end + 1;
                    continue;
                }
            }
        }
        if tail.starts_with(b"\x1b]10;") || tail.starts_with(b"\x1b]11;") {
            let bel = tail.iter().position(|b| *b == 7).map(|i| (i, 1));
            let st = tail.windows(2).position(|b| b == b"\x1b\\").map(|i| (i, 2));
            let end = match (bel, st) {
                (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
                (a, b) => a.or(b),
            };
            if let Some((end, len)) = end {
                if let Some(rgb) = parse_rgb(&String::from_utf8_lossy(&tail[5..end])) {
                    if tail[3] == b'0' {
                        caps.foreground = rgb;
                    } else {
                        caps.background = rgb;
                    }
                    pos += end + len;
                    continue;
                }
            }
        }
        caps.pending.push(bytes[pos]);
        pos += 1;
    }
    caps
}
fn parse_rgb(value: &str) -> Option<Rgb> {
    let mut parts = value.strip_prefix("rgb:")?.split('/');
    let mut channel = || {
        let v = parts.next()?;
        if v.is_empty() || v.len() > 4 {
            return None;
        }
        let n = u32::from_str_radix(v, 16).ok()?;
        Some((n * 255 / ((1u32 << (v.len() * 4)) - 1)) as u8)
    };
    let rgb = Rgb {
        r: channel()?,
        g: channel()?,
        b: channel()?,
    };
    parts.next().is_none().then_some(rgb)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_preserves_user_bytes() {
        let c = parse_capabilities(b"hello\x1b[?0u\x1b]10;rgb:ffff/8080/0000\x1b\\world");
        assert!(c.kitty);
        assert_eq!(c.pending, b"helloworld");
        assert_eq!(
            c.foreground,
            Rgb {
                r: 255,
                g: 128,
                b: 0
            }
        );
    }

    #[test]
    fn partial_and_malformed_replies_remain_input() {
        let bytes = b"a\x1b\x1b[?0\x1b]10;rgb:zz/00/00\x07\x1b]11;rgb:00/00/00/ff\x1b\\";
        let c = parse_capabilities(bytes);
        assert!(!c.kitty);
        assert_eq!(c.pending, bytes);
    }

    #[test]
    fn earliest_osc_terminator_and_interleaved_keys() {
        let c = parse_capabilities(b"a\x1b]11;rgb:00/10/ff\x1b\\b\x07\x1b[?1u\x1b[C");
        assert!(c.kitty);
        assert_eq!(
            c.background,
            Rgb {
                r: 0,
                g: 16,
                b: 255
            }
        );
        assert_eq!(c.pending, b"ab\x07\x1b[C");
    }
}
