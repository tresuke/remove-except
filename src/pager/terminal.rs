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
        let input = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Cannot open the controlling terminal for viewer input: {error}"),
                )
            })?;
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
        GetConsoleMode, INPUT_RECORD, KEY_EVENT, LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED,
        PeekConsoleInputW, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED, ReadConsoleInputW, SHIFT_PRESSED,
        SetConsoleMode,
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
        // UnicodeChar is the UTF-16 variant of the console key event.
        let character = unsafe { event.uChar.UnicodeChar };
        Ok(translate_key(
            event.bKeyDown != 0,
            event.wVirtualKeyCode,
            character,
            event.dwControlKeyState,
        ))
    }

    /// Converts one console key event into a viewer key.
    ///
    /// Precedence: key releases are ignored; character 3 is always Ctrl+C; keys
    /// without a mapping or character (modifier-only and function keys) are ignored
    /// without a redraw; left or right Alt without any Ctrl yields `Key::Unknown`, so
    /// Alt+c and Alt+q never act as viewer commands; Ctrl+Alt (including AltGr, which
    /// arrives as right Alt plus left Ctrl) keeps its typed character.
    pub(super) fn translate_key(
        key_down: bool,
        virtual_key: u16,
        character: u16,
        control_state: u32,
    ) -> Option<Key> {
        if !key_down {
            return None;
        }
        if character == 3 {
            return Some(Key::CtrlC);
        }
        let navigation = match virtual_key {
            0x26 => Some(Key::ArrowUp),
            0x28 => Some(Key::ArrowDown),
            0x21 => Some(Key::PageUp),
            0x22 => Some(Key::PageDown),
            0x24 => Some(Key::Home),
            0x23 => Some(Key::End),
            0x1b => Some(Key::Escape),
            0x0d => Some(Key::Enter),
            0x09 if control_state & SHIFT_PRESSED != 0 => Some(Key::BackTab),
            0x09 => Some(Key::Tab),
            _ => None,
        };
        if navigation.is_none() && character == 0 {
            return None;
        }
        let alt = control_state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0;
        let ctrl = control_state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0;
        if alt && !ctrl {
            return Some(Key::Unknown);
        }
        Some(navigation.unwrap_or_else(|| {
            char::from_u32(u32::from(character)).map_or(Key::Unknown, Key::Char)
        }))
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
            // Raw mode delivers Ctrl+D as an ordinary byte; only poll/read report a
            // closed terminal.
            4 => Key::Unknown,
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::platform::translate_key;
    use console::Key;
    use windows_sys::Win32::System::Console::{
        LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED, SHIFT_PRESSED,
    };

    fn letter(character: char, state: u32) -> Option<Key> {
        let virtual_key = character.to_ascii_uppercase() as u16;
        translate_key(true, virtual_key, character as u16, state)
    }

    #[test]
    fn releases_and_keys_without_characters_are_ignored() {
        assert_eq!(translate_key(false, 0x43, u16::from(b'c'), 0), None);
        assert_eq!(translate_key(false, 0x23, 0, 0), None);
        // Shift, Ctrl, Alt, Caps Lock, and F1 pressed on their own.
        for (virtual_key, state) in [
            (0x10, SHIFT_PRESSED),
            (0x11, LEFT_CTRL_PRESSED),
            (0x12, LEFT_ALT_PRESSED),
            (0x14, 0),
            (0x70, 0),
        ] {
            assert_eq!(translate_key(true, virtual_key, 0, state), None);
        }
    }

    #[test]
    fn alt_without_ctrl_never_produces_viewer_commands() {
        for alt in [LEFT_ALT_PRESSED, RIGHT_ALT_PRESSED] {
            for character in ['c', 'q', 'y'] {
                assert_eq!(letter(character, alt), Some(Key::Unknown));
                assert_eq!(letter(character, alt | SHIFT_PRESSED), Some(Key::Unknown));
            }
            assert_eq!(translate_key(true, 0x23, 0, alt), Some(Key::Unknown));
            assert_eq!(translate_key(true, 0x09, 9, alt), Some(Key::Unknown));
        }
    }

    #[test]
    fn ctrl_alt_and_altgr_keep_typed_characters() {
        // AltGr arrives as right Alt plus left Ctrl; any Ctrl+Alt pair is treated alike.
        for state in [
            RIGHT_ALT_PRESSED | LEFT_CTRL_PRESSED,
            LEFT_ALT_PRESSED | RIGHT_CTRL_PRESSED,
            LEFT_ALT_PRESSED | LEFT_CTRL_PRESSED,
            RIGHT_ALT_PRESSED | RIGHT_CTRL_PRESSED,
        ] {
            assert_eq!(
                translate_key(true, 0x51, u16::from(b'@'), state),
                Some(Key::Char('@'))
            );
            assert_eq!(
                translate_key(true, 0x45, 0x20ac, state),
                Some(Key::Char('\u{20ac}'))
            );
        }
    }

    #[test]
    fn ctrl_c_shift_tab_and_plain_characters_are_preserved() {
        for state in [
            LEFT_CTRL_PRESSED,
            RIGHT_CTRL_PRESSED,
            LEFT_CTRL_PRESSED | LEFT_ALT_PRESSED,
            LEFT_ALT_PRESSED,
        ] {
            assert_eq!(translate_key(true, 0x43, 3, state), Some(Key::CtrlC));
        }
        assert_eq!(translate_key(true, 0x09, 9, 0), Some(Key::Tab));
        assert_eq!(
            translate_key(true, 0x09, 9, SHIFT_PRESSED),
            Some(Key::BackTab)
        );
        assert_eq!(translate_key(true, 0x23, 0, 0), Some(Key::End));
        assert_eq!(translate_key(true, 0x1b, 0x1b, 0), Some(Key::Escape));
        assert_eq!(letter('c', 0), Some(Key::Char('c')));
        assert_eq!(letter('q', 0), Some(Key::Char('q')));
        assert_eq!(letter('C', SHIFT_PRESSED), Some(Key::Char('C')));
        assert_eq!(
            translate_key(true, 0x44, 4, LEFT_CTRL_PRESSED),
            Some(Key::Char('\x04'))
        );
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::FromRawFd;

    fn pipe() -> (File, File) {
        let mut descriptors = [0; 2];
        // pipe writes two new owned descriptors into the two-element array.
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        // Each descriptor is transferred to exactly one File.
        unsafe {
            (
                File::from_raw_fd(descriptors[0]),
                File::from_raw_fd(descriptors[1]),
            )
        }
    }

    #[test]
    fn ctrl_d_is_an_ignored_key_but_a_closed_input_is_end_of_file() {
        let (reader, mut writer) = pipe();
        writer.write_all(b"\x04q").unwrap();
        assert_eq!(platform::read_key(&reader).unwrap(), Some(Key::Unknown));
        assert_eq!(platform::read_key(&reader).unwrap(), Some(Key::Char('q')));
        drop(writer);
        let error = platform::read_key(&reader).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }
}
