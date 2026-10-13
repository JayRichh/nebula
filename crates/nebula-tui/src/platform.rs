//! The client's end of the daemon endpoint: a unix socket at
//! `paths::socket_path`, or on Windows the named pipe that stands in for it
//! (`paths::pipe_name_for_socket`). Either way a [`Stream`] is
//! `AsyncRead + AsyncWrite`, and [`split`] gives the reader and writer
//! halves the IPC tasks own.

use std::path::Path;

#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;

#[cfg(windows)]
pub type Stream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Connect to the daemon listening at `sock`.
#[cfg(unix)]
pub async fn connect(sock: &Path) -> std::io::Result<Stream> {
    tokio::net::UnixStream::connect(sock).await
}

/// Connect to the daemon serving the pipe that stands in for `sock`. Every
/// instance can be briefly busy between one client and the next instance
/// (`ERROR_PIPE_BUSY`), which is worth a short retry; a pipe that does not
/// exist means no daemon, and answers at once.
#[cfg(windows)]
pub async fn connect(sock: &Path) -> std::io::Result<Stream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
    let name = nebula_core::paths::pipe_name_for_socket(sock);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match ClientOptions::new().open(&name) {
            Ok(client) => return Ok(client),
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// The stream's reader and writer halves, each owned by its own task.
#[cfg(unix)]
pub fn split(
    stream: Stream,
) -> (
    tokio::net::unix::OwnedReadHalf,
    tokio::net::unix::OwnedWriteHalf,
) {
    stream.into_split()
}

#[cfg(windows)]
pub fn split(stream: Stream) -> (tokio::io::ReadHalf<Stream>, tokio::io::WriteHalf<Stream>) {
    tokio::io::split(stream)
}

/// The pid of the daemon serving the pipe, asked of the OS
/// (`GetNamedPipeServerProcessId`) — the pipe's counterpart of a socket's
/// peer credentials.
#[cfg(windows)]
pub fn server_pid(stream: &Stream) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let mut pid = 0u32;
    // SAFETY: the handle is a live pipe client for the call.
    let ok = unsafe { GetNamedPipeServerProcessId(stream.as_raw_handle(), &mut pid) } != 0;
    (ok && pid != 0).then_some(pid)
}

/// The full path of the executable `pid` runs, when this user may ask.
#[cfg(windows)]
pub fn process_exe_path(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: plain FFI; a null handle is the failure value.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units; the handle is live.
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
    } != 0;
    // SAFETY: opened above, closed once.
    unsafe { CloseHandle(process) };
    ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
}
