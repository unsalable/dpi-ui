//! Overlapped (eszamansiz) named pipe G/C.
//!
//! Senkron acilmis bir pipe tutamacinda Windows ayni dosya nesnesi uzerindeki
//! islemleri siraya koyar: bir is parcacigi `ReadFile`'da beklerken digerinin
//! `WriteFile`'i de bekler ve istemci yanit bekledigi icin kilitlenme olur.
//! Overlapped tutamacta okuma ve yazma birbirinden bagimsizdir; her biri kendi
//! olayiyla tamamlanmayi bekler.

use std::io::{self, Read, Write};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_NOT_CONNECTED,
    GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, ReadFile, WriteFile};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::Threading::CreateEventW;

/// Sahip olunan pipe tutamaci (dusurulunce kapanir).
pub struct PipeHandle(HANDLE);

unsafe impl Send for PipeHandle {}
unsafe impl Sync for PipeHandle {}

impl PipeHandle {
    /// # Safety
    /// `h`, FILE_FLAG_OVERLAPPED ile acilmis gecerli bir pipe tutamaci olmali.
    pub unsafe fn from_raw(h: HANDLE) -> Self {
        Self(h)
    }

    pub fn raw(&self) -> HANDLE {
        self.0
    }

    /// Istemci olarak baglanir; tum ornekler mesgulse `timeout_ms` kadar bekler.
    pub fn connect(name: &str, timeout_ms: u32) -> io::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        for _ in 0..2 {
            let h = unsafe {
                CreateFileW(
                    wide.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    std::ptr::null_mut(),
                )
            };
            if h != INVALID_HANDLE_VALUE {
                return Ok(Self(h));
            }
            if unsafe { GetLastError() } != ERROR_PIPE_BUSY {
                return Err(io::Error::last_os_error());
            }
            unsafe { WaitNamedPipeW(wide.as_ptr(), timeout_ms) };
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "pipe meşgul"))
    }

    /// Bu tutamactaki bekleyen tum okuma/yazmalari iptal eder (kapatma icin).
    pub fn cancel(&self) {
        unsafe { CancelIoEx(self.0, std::ptr::null()) };
    }

    /// Okuyucu ve yaziciya ayirir; ikisi farkli is parcaciklarinda ayni anda kullanilabilir.
    pub fn split(self) -> io::Result<(PipeReader, PipeWriter)> {
        let pipe = Arc::new(self);
        Ok((PipeReader { io: Overlapped::new(pipe.clone())? }, PipeWriter { io: Overlapped::new(pipe)? }))
    }
}

impl Drop for PipeHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct Overlapped {
    pipe: Arc<PipeHandle>,
    event: HANDLE,
}

unsafe impl Send for Overlapped {}

impl Overlapped {
    fn new(pipe: Arc<PipeHandle>) -> io::Result<Self> {
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { pipe, event })
    }

    fn run(&mut self, start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> i32) -> io::Result<usize> {
        let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
        ov.hEvent = self.event;
        let h = self.pipe.raw();
        if start(h, &mut ov) == 0 {
            let err = unsafe { GetLastError() };
            if err != ERROR_IO_PENDING {
                return map_err(err);
            }
        }
        let mut n = 0u32;
        if unsafe { GetOverlappedResult(h, &ov, &mut n, 1) } == 0 {
            return map_err(unsafe { GetLastError() });
        }
        Ok(n as usize)
    }

    pub fn pipe(&self) -> &PipeHandle {
        &self.pipe
    }
}

impl Drop for Overlapped {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.event) };
    }
}

fn map_err(code: u32) -> io::Result<usize> {
    match code {
        // Karsi taraf kapatti ya da iptal edildi: akis sonu.
        ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_OPERATION_ABORTED => Ok(0),
        c => Err(io::Error::from_raw_os_error(c as i32)),
    }
}

pub struct PipeReader {
    io: Overlapped,
}

impl PipeReader {
    pub fn pipe(&self) -> &PipeHandle {
        self.io.pipe()
    }
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_mut_ptr();
        self.io.run(|h, ov| unsafe { ReadFile(h, ptr, len, std::ptr::null_mut(), ov) })
    }
}

pub struct PipeWriter {
    io: Overlapped,
}

impl PipeWriter {
    pub fn pipe(&self) -> &PipeHandle {
        self.io.pipe()
    }
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_ptr();
        let n = self.io.run(|h, ov| unsafe { WriteFile(h, ptr, len, std::ptr::null_mut(), ov) })?;
        if n == 0 && !buf.is_empty() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "pipe kapandı"));
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
