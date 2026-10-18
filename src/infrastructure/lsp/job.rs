//! Ends a server's whole process tree with it.
//!
//! A language server starts helpers of its own (rust-analyzer runs proc-macro
//! servers and cargo). On Windows a process added to a job object marked
//! kill-on-close takes its children down when the job handle closes, including
//! when Loom itself dies. Elsewhere servers are left to exit on their closed
//! standard input.
#[cfg(windows)]
pub(crate) struct Job(windows_sys::Win32::Foundation::HANDLE);

// The handle is only closed on drop and never shared.
#[cfg(windows)]
unsafe impl Send for Job {}

#[cfg(windows)]
impl Job {
    /// Puts `process` into a new kill-on-close job. `None` if Windows refuses, for
    /// example when the process already belongs to a job that forbids nesting.
    pub fn adopt(process: &std::process::Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        // SAFETY: plain Win32 calls on handles this function creates or borrows
        // from `process`, which outlives the calls.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0;
            if !configured || AssignProcessToJobObject(job, process.as_raw_handle() as _) == 0 {
                windows_sys::Win32::Foundation::CloseHandle(job);
                return None;
            }
            Some(Self(job))
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
pub(crate) struct Job;

#[cfg(not(windows))]
impl Job {
    pub fn adopt(_process: &std::process::Child) -> Option<Self> {
        None
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn closing_the_job_ends_the_process_and_what_it_started() {
        // cmd starts ping, which would run for half a minute.
        let mut child = Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let job = Job::adopt(&child).expect("a plain process can join a job");
        std::thread::sleep(Duration::from_millis(300));
        assert!(child.try_wait().unwrap().is_none(), "still running");
        drop(job);
        let limit = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < limit, "the process survived its job");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
