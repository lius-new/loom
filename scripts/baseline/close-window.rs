//! Request an orderly GUI shutdown for one explicitly identified child.
//! Standalone rustc input: no Cargo dependencies and no PowerShell runtime.
#![cfg_attr(windows, windows_subsystem = "console")]

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::path::Path;

    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(
            callback: unsafe extern "system" fn(Handle, isize) -> i32,
            data: isize,
        ) -> i32;
        fn GetWindowThreadProcessId(window: Handle, pid: *mut u32) -> u32;
        fn GetWindow(window: Handle, command: u32) -> Handle;
        fn IsWindowVisible(window: Handle) -> i32;
        fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
    }
    struct Process(Handle);
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct Search {
        pid: u32,
        window: Handle,
    }
    unsafe extern "system" fn visit(window: Handle, data: isize) -> i32 {
        let search = unsafe { &mut *(data as *mut Search) };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
        }
        if pid == search.pid && unsafe { GetWindow(window, 4) }.is_null() {
            // GW_OWNER
            search.window = window;
            if unsafe { IsWindowVisible(window) } != 0 {
                return 0;
            }
        }
        1
    }
    fn normalized(path: &Path) -> String {
        let text = path.to_string_lossy().replace('/', "\\");
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}").to_lowercase()
        } else {
            text.strip_prefix(r"\\?\").unwrap_or(&text).to_lowercase()
        }
    }
    pub fn close(pid: u32, executable: &Path) -> Result<(), String> {
        if pid == 0 {
            return Err("Refusing PID zero".into());
        }
        let process = Process(unsafe { OpenProcess(0x1000, 0, pid) }); // QUERY_LIMITED_INFORMATION
        if process.0.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut length) }
            == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let actual = String::from_utf16_lossy(&buffer[..length as usize]);
        let expected = std::fs::canonicalize(executable).map_err(|e| e.to_string())?;
        if normalized(Path::new(&actual)) != normalized(&expected) {
            return Err("Process executable does not match the smoke child".into());
        }
        // Keep the process handle open while locating and messaging its window.
        let mut search = Search {
            pid,
            window: std::ptr::null_mut(),
        };
        unsafe {
            EnumWindows(visit, (&mut search as *mut Search) as isize);
        }
        if search.window.is_null() {
            return Err("No main window for the smoke child".into());
        }
        // Recheck ownership immediately before posting. Never broadcast or kill.
        let mut owner = 0;
        unsafe {
            GetWindowThreadProcessId(search.window, &mut owner);
        }
        if owner != pid {
            return Err("Window ownership changed".into());
        }
        if unsafe { PostMessageW(search.window, 0x0010, 0, 0) } == 0 {
            // WM_CLOSE
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
}

fn main() {
    #[cfg(windows)]
    {
        let args: Vec<_> = std::env::args_os().collect();
        let result = if args.len() == 3 {
            args[1]
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or_else(|| "Invalid PID".to_string())
                .and_then(|pid| windows::close(pid, std::path::Path::new(&args[2])))
        } else {
            Err("Usage: close-window <pid> <expected-executable>".into())
        };
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("This helper requires Windows");
        std::process::exit(1);
    }
}
