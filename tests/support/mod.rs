use std::ffi::OsString;

pub fn non_unicode_name(byte: u8) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec([b"bad-".as_slice(), &[byte], b".txt"].concat())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut name: Vec<u16> = "bad-".encode_utf16().collect();
        name.push(0xd800 + u16::from(byte));
        name.extend(".txt".encode_utf16());
        OsString::from_wide(&name)
    }
}
