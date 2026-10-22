use std::fs::{File, OpenOptions};
use std::io;

use console::Key;

pub(super) struct Terminal {
    input: File,
    #[cfg(windows)]
    original: u32,
    #[cfg(unix)]
    original: libc::termios,
    active: bool,
}

impl Terminal {
    pub(super) fn open() -> io::Result<Self> {
        #[cfg(windows)]
        let path = "CONIN$";
        #[cfg(unix)]
        let path = "/dev/tty";
        let input = OpenOptions::new().read(true).write(true).open(path)?;
        let original = platform::mode(&input)?;
        Ok(Self {
            input,
            original,
            active: false,
        })
    }

    pub(super) fn start(&mut self) -> io::Result<()> {
        // Mark active before changing the mode so a partial failure also restores it.
        self.active = true;
        platform::start(&self.input, &self.original)?;
        self.discard_input()
    }

    pub(super) fn read_key(&self) -> io::Result<Option<Key>> {
        platform::read_key(&self.input)
    }

    pub(super) fn discard_input(&self) -> io::Result<()> {
        platform::discard_input(&self.input)
    }

    pub(super) fn finish(&mut self) -> io::Result<()> {
        if self.active {
            let discarded = self.discard_input();
            let restored = platform::restore(&self.input, &self.original);
            if restored.is_ok() {
                self.active = false;
            }
            match (discarded, restored) {
                (Ok(()), Ok(())) => {}
                (discarded, restored) => {
                    return Err(io::Error::other(format!(
                        "Terminal cleanup failed: input={discarded:?}, mode={restored:?}"
                    )));
                }
            }
        }
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if self.active {
            // Normal exits use finish(), where errors can prevent deletion.
            if let Err(error) = platform::restore(&self.input, &self.original) {
                eprintln!("Failed to restore viewer terminal input: {error}");
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, FlushConsoleInputBuffer,
        GetConsoleMode, INPUT_RECORD, KEY_EVENT, PeekConsoleInputW, ReadConsoleInputW,
        SHIFT_PRESSED, SetConsoleMode,
    };
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    pub(super) fn mode(input: &File) -> io::Result<u32> {
        let mut mode = 0;
        // The file owns a valid console input handle; mode is writable for this call.
        if unsafe { GetConsoleMode(input.as_raw_handle(), &mut mode) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(mode)
    }

    pub(super) fn start(input: &File, original: &u32) -> io::Result<()> {
        restore(
            input,
            &(original & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT)),
        )
    }

    pub(super) fn restore(input: &File, mode: &u32) -> io::Result<()> {
        // The owned handle stays open throughout the synchronous API call.
        if unsafe { SetConsoleMode(input.as_raw_handle(), *mode) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn discard_input(input: &File) -> io::Result<()> {
        // Only the dedicated console input handle is flushed, never pattern stdin.
        if unsafe { FlushConsoleInputBuffer(input.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn read_key(input: &File) -> io::Result<Option<Key>> {
        // The console handle is waitable and remains owned by input.
        match unsafe { WaitForSingleObject(input.as_raw_handle(), 100) } {
            WAIT_TIMEOUT => return Ok(None),
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => {}
        }
        let mut record = INPUT_RECORD::default();
        let mut read = 0;
        // A signaled console handle can have no readable records after mode/event
        // processing. Do not enter a blocking read and stop resize polling.
        if unsafe { PeekConsoleInputW(input.as_raw_handle(), &mut record, 1, &mut read) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if read == 0 {
            return Ok(None);
        }
        // record has space for exactly the one requested console record.
        if unsafe { ReadConsoleInputW(input.as_raw_handle(), &mut record, 1, &mut read) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Console input closed",
            ));
        }
        if record.EventType != KEY_EVENT as u16 {
            return Ok(None);
        }
        // EventType identifies the active union member.
        let event = unsafe { record.Event.KeyEvent };
        if event.bKeyDown == 0 {
            return Ok(None);
        }
        // UnicodeChar is the UTF-16 variant of the console key event.
        let character = unsafe { event.uChar.UnicodeChar };
        let key = match event.wVirtualKeyCode {
            0x26 => Key::ArrowUp,
            0x28 => Key::ArrowDown,
            0x21 => Key::PageUp,
            0x22 => Key::PageDown,
            0x24 => Key::Home,
            0x23 => Key::End,
            0x1b => Key::Escape,
            0x0d => Key::Enter,
            0x09 if event.dwControlKeyState & SHIFT_PRESSED != 0 => Key::BackTab,
            0x09 => Key::Tab,
            _ if character == 3 => Key::CtrlC,
            _ => char::from_u32(u32::from(character)).map_or(Key::Unknown, Key::Char),
        };
        Ok(Some(key))
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::fd::AsRawFd;

    pub(super) fn mode(input: &File) -> io::Result<libc::termios> {
        let mut mode = std::mem::MaybeUninit::uninit();
        // tcgetattr initializes the complete termios value on success.
        if unsafe { libc::tcgetattr(input.as_raw_fd(), mode.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { mode.assume_init() })
    }

    pub(super) fn start(input: &File, original: &libc::termios) -> io::Result<()> {
        let mut mode = *original;
        // mode is an initialized termios value. Preserve output processing on this TTY.
        unsafe { libc::cfmakeraw(&mut mode) };
        mode.c_oflag = original.c_oflag;
        restore(input, &mode)
    }

    pub(super) fn restore(input: &File, mode: &libc::termios) -> io::Result<()> {
        // input owns the TTY fd and mode is initialized for this platform.
        if unsafe { libc::tcsetattr(input.as_raw_fd(), libc::TCSANOW, mode) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn discard_input(input: &File) -> io::Result<()> {
        // Discard only pending control-terminal input, not stdin's patterns.
        if unsafe { libc::tcflush(input.as_raw_fd(), libc::TCIFLUSH) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn read_byte(input: &File, timeout: i32) -> io::Result<Option<u8>> {
        let mut descriptor = libc::pollfd {
            fd: input.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // descriptor is a valid one-element pollfd array.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Ok(None);
        }
        if descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Terminal input closed",
            ));
        }
        let mut byte = 0;
        // byte provides exactly the requested single byte of writable storage.
        match unsafe { libc::read(input.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) } {
            1 => Ok(Some(byte)),
            0 => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Terminal input closed",
            )),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn read_key(input: &File) -> io::Result<Option<Key>> {
        let Some(byte) = read_byte(input, 100)? else {
            return Ok(None);
        };
        let key = match byte {
            3 => Key::CtrlC,
            4 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Terminal input ended",
                ));
            }
            b'\r' | b'\n' => Key::Enter,
            b'\t' => Key::Tab,
            0x1b => {
                let Some(next) = read_byte(input, 30)? else {
                    return Ok(Some(Key::Escape));
                };
                if next != b'[' && next != b'O' {
                    return Ok(Some(Key::Unknown));
                }
                let mut sequence = Vec::new();
                while let Some(next) = read_byte(input, 30)? {
                    sequence.push(next);
                    if next.is_ascii_alphabetic() || next == b'~' || sequence.len() >= 16 {
                        break;
                    }
                }
                match sequence.as_slice() {
                    b"A" => Key::ArrowUp,
                    b"B" => Key::ArrowDown,
                    b"H" | b"1~" | b"7~" => Key::Home,
                    b"F" | b"4~" | b"8~" => Key::End,
                    b"5~" => Key::PageUp,
                    b"6~" => Key::PageDown,
                    b"Z" => Key::BackTab,
                    _ => Key::Unknown,
                }
            }
            byte if byte.is_ascii() => Key::Char(char::from(byte)),
            _ => Key::Unknown,
        };
        Ok(Some(key))
    }
}
