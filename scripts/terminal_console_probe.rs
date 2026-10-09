//! Read-only observer for one Windows pseudoconsole. Kept out of the editor
//! process because AttachConsole changes the calling process's console.
#![windows_subsystem = "windows"]

use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::time::Duration;
use std::time::Instant;

type Handle = *mut c_void;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn AttachConsole(pid: u32) -> i32;
    fn FreeConsole() -> i32;
    fn GetConsoleProcessList(ids: *mut u32, count: u32) -> u32;
    fn GetStdHandle(kind: u32) -> Handle;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}

unsafe extern "system" fn ignore_control(_event: u32) -> i32 {
    1
}

fn main() {
    let Some(root) = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse::<u32>().ok())
    else {
        return;
    };
    // Preserve the parent's output pipe before attaching to the target console.
    let pipe = unsafe { GetStdHandle((-11i32) as u32) };
    if pipe.is_null() || pipe as isize == -1 {
        return;
    }
    let mut output = unsafe { File::from_raw_handle(pipe) };
    let root_handle = unsafe { OpenProcess(0x0010_0000, 0, root) }; // SYNCHRONIZE
    if root_handle.is_null() {
        return;
    }
    let _root_handle = unsafe { OwnedHandle::from_raw_handle(root_handle) };
    // The launched PID may be a bootstrapper which attaches its console after
    // CreateProcess returns. Also discard any inherited observer console.
    unsafe {
        FreeConsole();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { AttachConsole(root) } == 0 {
        if Instant::now() >= deadline || unsafe { WaitForSingleObject(root_handle, 0) } != 258 {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    unsafe {
        SetConsoleCtrlHandler(Some(ignore_control), 1);
    }
    let own = std::process::id();
    let mut ids = vec![0u32; 32];
    while unsafe { WaitForSingleObject(root_handle, 0) } == 258 {
        // WAIT_TIMEOUT
        let count = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
        if count == 0 {
            break;
        }
        if count as usize > ids.len() {
            ids.resize(count as usize, 0);
            continue;
        }
        let line = ids[..count as usize]
            .iter()
            .filter(|id| **id != own)
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if writeln!(output, "{line}")
            .and_then(|_| output.flush())
            .is_err()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}
