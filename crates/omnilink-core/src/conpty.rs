#[cfg(windows)]
pub mod win {
    use std::{
        ffi::c_void,
        io,
        mem,
        ptr,
    };
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            Security::SECURITY_ATTRIBUTES,
            Storage::FileSystem::{ReadFile, WriteFile},
            System::{
                Console::{
                    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
                },
                Pipes::CreatePipe,
                Threading::{
                    CreateProcessW, DeleteProcThreadAttributeList,
                    InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
                    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, STARTUPINFOEXW,
                },
            },
        },
    };

    const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x00020016;
    const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x00080000;

    pub struct PseudoConsole {
        hpcon: HPCON,
        process_info: PROCESS_INFORMATION,
        pipe_in: HANDLE,
        pipe_out: HANDLE,
    }

    // Safety: Handles can be sent across threads
    unsafe impl Send for PseudoConsole {}
    unsafe impl Sync for PseudoConsole {}

    impl PseudoConsole {
        pub fn new(command: &str, cols: i16, rows: i16) -> io::Result<Self> {
            unsafe {
                let mut h_pipe_pty_in = HANDLE::default();
                let mut h_pipe_pty_out = HANDLE::default();
                let mut h_pipe_in = HANDLE::default();
                let mut h_pipe_out = HANDLE::default();

                let mut sa = SECURITY_ATTRIBUTES {
                    nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                    bInheritHandle: false.into(),
                    lpSecurityDescriptor: ptr::null_mut(),
                };

                // Create input pipe (App writes to h_pipe_in, ConPTY reads from h_pipe_pty_in)
                if CreatePipe(&mut h_pipe_pty_in, &mut h_pipe_in, Some(&mut sa), 0).is_err() {
                    return Err(io::Error::last_os_error());
                }

                // Create output pipe (ConPTY writes to h_pipe_pty_out, App reads from h_pipe_out)
                if CreatePipe(&mut h_pipe_out, &mut h_pipe_pty_out, Some(&mut sa), 0).is_err() {
                    let _ = CloseHandle(h_pipe_pty_in);
                    let _ = CloseHandle(h_pipe_in);
                    return Err(io::Error::last_os_error());
                }

                let size = COORD { X: cols, Y: rows };
                let hpcon = match CreatePseudoConsole(size, h_pipe_pty_in, h_pipe_pty_out, 0) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = CloseHandle(h_pipe_pty_in);
                        let _ = CloseHandle(h_pipe_in);
                        let _ = CloseHandle(h_pipe_pty_out);
                        let _ = CloseHandle(h_pipe_out);
                        return Err(io::Error::new(io::ErrorKind::Other, format!("CreatePseudoConsole failed: {:?}", e)));
                    }
                };

                // Initialize ProcThreadAttributeList
                let mut attr_size: usize = 0;
                let _ = InitializeProcThreadAttributeList(LPPROC_THREAD_ATTRIBUTE_LIST(ptr::null_mut()), 1, 0, &mut attr_size);

                let mut attr_list_buf = vec![0u8; attr_size];
                let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(attr_list_buf.as_mut_ptr() as *mut c_void);
                if InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size).is_err() {
                    let _ = CloseHandle(h_pipe_pty_in);
                    let _ = CloseHandle(h_pipe_pty_out);
                    let _ = ClosePseudoConsole(hpcon);
                    return Err(io::Error::last_os_error());
                }

                if UpdateProcThreadAttribute(
                    attr_list,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    Some(&hpcon as *const HPCON as *const c_void),
                    mem::size_of::<HPCON>(),
                    None,
                    None,
                ).is_err() {
                    let _ = CloseHandle(h_pipe_pty_in);
                    let _ = CloseHandle(h_pipe_pty_out);
                    DeleteProcThreadAttributeList(attr_list);
                    let _ = ClosePseudoConsole(hpcon);
                    return Err(io::Error::last_os_error());
                }

                let mut startup_info_ex: STARTUPINFOEXW = mem::zeroed();
                startup_info_ex.StartupInfo.cb = mem::size_of::<STARTUPINFOEXW>() as u32;
                startup_info_ex.lpAttributeList = attr_list;

                let mut cmd_wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
                let mut process_info: PROCESS_INFORMATION = mem::zeroed();

                let created = CreateProcessW(
                    None,
                    PWSTR(cmd_wide.as_mut_ptr()),
                    None,
                    None,
                    false,
                    windows::Win32::System::Threading::PROCESS_CREATION_FLAGS(EXTENDED_STARTUPINFO_PRESENT),
                    None,
                    None,
                    &startup_info_ex.StartupInfo,
                    &mut process_info,
                );

                DeleteProcThreadAttributeList(attr_list);
                let _ = CloseHandle(h_pipe_pty_in);
                let _ = CloseHandle(h_pipe_pty_out);

                if created.is_err() {
                    let _ = ClosePseudoConsole(hpcon);
                    return Err(io::Error::last_os_error());
                }

                Ok(Self {
                    hpcon,
                    process_info,
                    pipe_in: h_pipe_in,
                    pipe_out: h_pipe_out,
                })
            }
        }

        pub fn resize(&self, cols: i16, rows: i16) -> io::Result<()> {
            unsafe {
                let size = COORD { X: cols, Y: rows };
                if let Err(e) = ResizePseudoConsole(self.hpcon, size) {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Resize failed: {:?}", e)));
                }
                Ok(())
            }
        }

        pub fn write_input(&self, data: &[u8]) -> io::Result<usize> {
            unsafe {
                let mut written = 0u32;
                if WriteFile(
                    self.pipe_in,
                    Some(data),
                    Some(&mut written),
                    None,
                ).is_ok() {
                    Ok(written as usize)
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        }

        pub fn read_output(&self, buf: &mut [u8]) -> io::Result<usize> {
            unsafe {
                let mut bytes_read = 0u32;
                if ReadFile(
                    self.pipe_out,
                    Some(buf),
                    Some(&mut bytes_read),
                    None,
                ).is_ok() {
                    Ok(bytes_read as usize)
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        }
    }

    impl Drop for PseudoConsole {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.pipe_in);
                let _ = CloseHandle(self.pipe_out);
                let _ = ClosePseudoConsole(self.hpcon);
                let _ = CloseHandle(self.process_info.hProcess);
                let _ = CloseHandle(self.process_info.hThread);
            }
        }
    }
}
