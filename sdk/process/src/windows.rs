//! Assign suspended children before their first instruction, then resume them.
use std::{
    io,
    mem::size_of,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Child, Command},
};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread,
            THREAD_SUSPEND_RESUME,
        },
    },
};

pub struct Job(OwnedHandle);
/// Cumulative user and kernel CPU time. FILETIME units are 100 ns; accounting
/// granularity depends on Windows and should not be confused with accuracy.
pub fn cpu_time(pid: u32) -> io::Result<std::time::Duration> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    // SAFETY: query-only access to the specified PID; no borrowed pointers.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcess returned a valid, uniquely owned handle.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: all four output buffers are initialized and valid; handle stays alive.
    if unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let ticks = |v: FILETIME| (u64::from(v.dwHighDateTime) << 32) | u64::from(v.dwLowDateTime);
    let ticks = ticks(kernel).saturating_add(ticks(user));
    Ok(std::time::Duration::new(
        ticks / 10_000_000,
        ((ticks % 10_000_000) * 100) as u32,
    ))
}

#[cfg(test)]
mod cpu_tests {
    #[test]
    fn reads_monotonic_cpu_and_closes_query_handles() {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
        let handles = || {
            let mut count = 0;
            // SAFETY: current-process pseudo-handle and a valid output pointer.
            assert_ne!(
                unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) },
                0
            );
            count
        };
        let before_handles = handles();
        let before = super::cpu_time(std::process::id()).expect("CPU time");
        for _ in 0..128 {
            assert!(super::cpu_time(std::process::id()).expect("CPU time") >= before);
        }
        // Other parallel tests may open handles; a leaked handle per sample would
        // exceed this allowance by an order of magnitude.
        assert!(handles() <= before_handles + 16);
        assert!(super::cpu_time(u32::MAX).is_err());
    }
}
impl Job {
    pub fn prepare(command: &mut Command) -> io::Result<Self> {
        // SAFETY: no security attributes/name; returned handle is uniquely owned.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a valid, uniquely owned handle.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: handle is valid; buffer and size match the selected information class.
        if unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        Ok(job)
    }
    pub fn attach_and_resume(&self, child: &Child) -> io::Result<()> {
        // SAFETY: both handles stay alive throughout assignment.
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // Stable std does not expose the primary thread handle. A suspended new
        // process has one thread; enumerate it only after it belongs to our Job.
        // SAFETY: system thread snapshot; no pointer arguments.
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: snapshot is a valid uniquely owned handle.
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut entry = THREADENTRY32 {
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        // SAFETY: writable entry has the required size; snapshot stays alive.
        let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == child.id() {
                // SAFETY: requests only resume permission for the observed thread.
                let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: OpenThread returned a valid uniquely owned handle.
                let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
                // SAFETY: valid thread handle with resume permission.
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            // SAFETY: same live snapshot and correctly sized entry as above.
            found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "suspended child thread not found",
        ))
    }
    pub fn is_empty(&self) -> io::Result<bool> {
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: owned live Job handle; the output buffer and size match the information class.
        if unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses == 0)
    }
    pub fn terminate(&self) {
        // SAFETY: live Job handle. All contained processes are ours.
        unsafe {
            TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
}

pub fn is_running(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{ERROR_INVALID_PARAMETER, GetLastError, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: only asks to observe process completion, never to mutate it.
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if raw.is_null() {
        // A missing PID is gone; inaccessible processes must not count as exited.
        // SAFETY: reads the error from the failed OpenProcess call above.
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    }
    // SAFETY: OpenProcess returned a valid uniquely owned handle.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: live process handle with synchronize access; zero timeout.
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0) == WAIT_TIMEOUT }
}
