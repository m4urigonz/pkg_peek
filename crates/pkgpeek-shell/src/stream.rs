//! `IStream` as `Read + Seek`, for handlers initialized with a stream.

use std::io::{self, Read, Seek, SeekFrom};

use windows::Win32::System::Com::{IStream, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET};

pub struct StreamReader(pub IStream);

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        let mut read = 0u32;
        let hr = unsafe { self.0.Read(buf.as_mut_ptr().cast(), len, Some(&mut read)) };
        if hr.is_err() {
            return Err(io::Error::other(windows_core::Error::from_hresult(hr)));
        }
        Ok(read as usize)
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let (offset, origin) = match pos {
            SeekFrom::Start(n) => (n as i64, STREAM_SEEK_SET),
            SeekFrom::End(n) => (n, STREAM_SEEK_END),
            SeekFrom::Current(n) => (n, STREAM_SEEK_CUR),
        };
        let mut position = 0u64;
        unsafe { self.0.Seek(offset, origin, Some(&mut position)) }.map_err(io::Error::other)?;
        Ok(position)
    }
}
