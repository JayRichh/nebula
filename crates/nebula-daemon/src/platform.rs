//! The daemon's OS seams: where it listens, what a session's process tree
//! is held by, and the fd type the IN-PLACE RESTART state names. Unix keeps
//! its socket, process groups and real fds; Windows gets a named pipe and a
//! job object per session, and has no in-place restart to carry fds across.

/// An fd an IN-PLACE RESTART keeps open across the exec. Windows has no
/// exec-over-self, so there it is only a placeholder that lets the restart
/// state's types compile; no Windows code path ever reads one.
#[cfg(unix)]
pub use std::os::fd::RawFd;
#[cfg(windows)]
pub type RawFd = i32;

#[cfg(windows)]
pub use self::windows::*;

#[cfg(windows)]
mod windows {
    use std::io;
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// The daemon's client endpoint: a named pipe (`paths::pipe_name`) that
    /// only this user can open and no remote machine can reach.
    ///
    /// A pipe server instance serves one client, so the listener always
    /// holds the next unconnected instance: [`Self::accept`] waits for a
    /// client on it, puts a fresh instance in its place, and hands the
    /// connected one off — there is never a moment with no instance to
    /// connect to.
    pub struct PipeListener {
        name: String,
        security: PipeSecurity,
        next: NamedPipeServer,
    }

    impl PipeListener {
        /// Create the pipe's first instance. `first_pipe_instance` makes the
        /// create fail when another process already serves the name — a
        /// second daemon, or something squatting on it — rather than
        /// quietly joining it.
        pub fn bind(name: &str) -> io::Result<Self> {
            let security = PipeSecurity::current_user()?;
            let next = create(name, &security, true)?;
            Ok(Self {
                name: name.to_string(),
                security,
                next,
            })
        }

        /// Wait for the next client and return its connected instance.
        pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
            let connected = self.next.connect().await;
            // Replace the instance whether or not the connect worked: one
            // whose client gave up mid-connect is no use to the next.
            let fresh = create(&self.name, &self.security, false)?;
            let served = std::mem::replace(&mut self.next, fresh);
            connected.map(|()| served)
        }
    }

    fn create(name: &str, security: &PipeSecurity, first: bool) -> io::Result<NamedPipeServer> {
        let mut attrs = security.attributes();
        let mut options = ServerOptions::new();
        options.first_pipe_instance(first).reject_remote_clients(true);
        // SAFETY: `attrs` points at a security descriptor `security` owns,
        // which outlives the call; the API copies what it needs.
        unsafe {
            options.create_with_security_attributes_raw(name, (&mut attrs as *mut SECURITY_ATTRIBUTES).cast())
        }
    }

    /// A security descriptor granting the current user, and only that user,
    /// full access — the DACL that makes the pipe the auth boundary the
    /// unix socket's 0700 runtime dir is. The default pipe DACL would also
    /// give Everyone read access.
    struct PipeSecurity(PSECURITY_DESCRIPTOR);

    // SAFETY: the descriptor is immutable once built, and freed only on drop.
    unsafe impl Send for PipeSecurity {}
    unsafe impl Sync for PipeSecurity {}

    impl PipeSecurity {
        fn current_user() -> io::Result<Self> {
            let sid = current_user_sid()?;
            // Protected DACL (`P`): one allow-all ACE for the user's SID.
            let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
                .encode_utf16()
                .chain([0])
                .collect();
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            // SAFETY: `sddl` is NUL-terminated; the descriptor is allocated by
            // the API and freed with LocalFree on drop.
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(descriptor))
        }

        fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.0,
                bInheritHandle: 0,
            }
        }
    }

    impl Drop for PipeSecurity {
        fn drop(&mut self) {
            // SAFETY: allocated by ConvertStringSecurityDescriptor…; freed once.
            unsafe { LocalFree(self.0 as _) };
        }
    }

    /// The current process's user SID in string form (`S-1-5-21-…`).
    fn current_user_sid() -> io::Result<String> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo-handle needs no closing; `token` is closed below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let mut len = 0u32;
            // SAFETY: a size query with no buffer; fails with the size needed.
            unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
            // u64 words keep the buffer aligned for TOKEN_USER.
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            // SAFETY: `buf` holds at least `len` bytes.
            if unsafe {
                GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len)
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: the call filled `buf` with a TOKEN_USER.
            let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
            let mut wide: windows_sys::core::PWSTR = std::ptr::null_mut();
            // SAFETY: the SID points into `buf`, alive for the call; `wide`
            // is allocated by the API and freed with LocalFree below.
            if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut wide) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `wide` is a NUL-terminated string from the API.
            let sid = unsafe {
                let len = (0..).take_while(|&i| *wide.add(i) != 0).count();
                String::from_utf16_lossy(std::slice::from_raw_parts(wide, len))
            };
            // SAFETY: freed once, as the API documents.
            unsafe { LocalFree(wide as _) };
            Ok(sid)
        })();
        // SAFETY: opened above, closed once.
        unsafe { CloseHandle(token) };
        result
    }

    /// The job object a session's process tree runs in: the Windows stand-in
    /// for the process groups `PtySession::kill` signals on unix. Everything
    /// the child starts joins the job, so one terminate reaches the whole
    /// tree, and closing the last handle — the session dropped, or the
    /// daemon gone — kills whatever is left (`KILL_ON_JOB_CLOSE`).
    pub struct Job(HANDLE);

    // SAFETY: a job handle may be used from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// Put `process` (a handle with set-quota and terminate rights, as a
        /// freshly spawned child's is) in a new kill-on-close job.
        pub fn for_process(process: std::os::windows::io::RawHandle) -> io::Result<Self> {
            // SAFETY: plain FFI call; a null handle is the failure value.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            // SAFETY: an all-zero limit struct is valid; one flag is set.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` is the struct the information class names.
            let ok = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: both handles are live for the call.
            if unsafe { AssignProcessToJobObject(job.0, process as HANDLE) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }

        /// End every process in the job.
        pub fn terminate(&self) -> io::Result<()> {
            // SAFETY: the handle is live until drop.
            if unsafe { TerminateJobObject(self.0, 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: created in `for_process`, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// `taskkill /T /F` over `pid`'s tree: the fallback for a session whose
    /// child could not be put in a job.
    pub fn kill_tree(pid: u32) {
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        nebula_core::platform::no_window(&mut cmd);
        let _ = cmd.status();
    }
}
