//! Killing a command together with everything it started.
//!
//! On Windows a test run's direct child is usually a launcher (`mvn.cmd`, `py.exe`, `make`), so
//! terminating only it leaves the student's program running and holding the output pipes.

/// Owns the Windows job object a command and its descendants run in; dropping it kills them.
/// On Unix the process group set up at spawn plays this role and this type does nothing.
pub(crate) struct ProcessTree {
    #[cfg(windows)]
    job: Option<windows_job::JobHandle>,
}

impl ProcessTree {
    /// Puts the already-started process `pid` into a new kill-on-close job. Failure is logged
    /// and leaves only the direct child killable.
    pub(crate) fn attach(pid: u32) -> Self {
        #[cfg(windows)]
        {
            let job = windows_job::JobHandle::attach(pid)
                .inspect_err(|err| {
                    log::warn!("failed to create job object for process {pid}: {err}")
                })
                .ok();
            Self { job }
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            Self {}
        }
    }

    /// Kills every process in the tree.
    pub(crate) fn kill(&self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
    }
}

#[cfg(windows)]
mod windows_job {
    use std::{io, mem};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject,
            },
            Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
        },
    };

    pub(super) struct JobHandle(HANDLE);

    // SAFETY: a job object handle may be used from any thread.
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    fn is_null(handle: HANDLE) -> bool {
        handle as usize == 0
    }

    impl JobHandle {
        pub(super) fn attach(pid: u32) -> io::Result<Self> {
            // SAFETY: plain Win32 calls; every handle is closed on each path.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if is_null(job) {
                    return Err(io::Error::last_os_error());
                }
                let job = Self(job);

                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }

                let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                if is_null(process) {
                    return Err(io::Error::last_os_error());
                }
                let assigned = AssignProcessToJobObject(job.0, process);
                let assign_error = io::Error::last_os_error();
                CloseHandle(process);
                if assigned == 0 {
                    return Err(assign_error);
                }
                Ok(job)
            }
        }

        pub(super) fn terminate(&self) {
            // SAFETY: the handle is valid until drop.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            // SAFETY: the handle is owned and closed once; closing kills the job's processes.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
