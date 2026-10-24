#[cfg(windows)]
pub mod win {
    use anyhow::{anyhow, Result};
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use windows::core::Interface;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };
    use wry::{WebView, WebViewExtWindows};

    pub struct Governor {
        job: HANDLE,
    }

    unsafe impl Send for Governor {}
    unsafe impl Sync for Governor {}

    impl Drop for Governor {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.job);
            }
        }
    }

    impl Governor {
        pub fn new(per_process_bytes: usize, total_bytes: usize) -> Result<Self> {
            unsafe {
                let job = CreateJobObjectW(None, windows::core::PCWSTR::null())?;
                if job.is_invalid() {
                    return Err(anyhow!("CreateJobObjectW returned an invalid handle"));
                }

                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
                    | JOB_OBJECT_LIMIT_JOB_MEMORY
                    | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                info.ProcessMemoryLimit = per_process_bytes;
                info.JobMemoryLimit = total_bytes;

                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )?;

                Ok(Self { job })
            }
        }

        pub fn attach(&self, pid: u32) -> Result<()> {
            unsafe {
                let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)?;
                if process.is_invalid() {
                    return Err(anyhow!("OpenProcess failed for PID {pid}"));
                }

                let assign_result = AssignProcessToJobObject(self.job, process);
                let _ = CloseHandle(process);
                assign_result?;
            }

            Ok(())
        }
    }

    pub fn enumerate_and_attach_renderers(
        governor: &Governor,
        webview: &WebView,
    ) -> Result<Vec<u32>> {
        let controller = webview.controller();
        let core = unsafe { controller.CoreWebView2()? };

        let core2: ICoreWebView2_2 = core.cast()?;
        let env = unsafe { core2.Environment()? };

        let env13: ICoreWebView2Environment13 = env.cast()?;

        let collection = unsafe { env13.GetProcessInfos()? };

        let mut count: u32 = 0;
        unsafe { collection.Count(&mut count)? };

        let mut attached = Vec::new();

        for i in 0..count {
            let info = unsafe { collection.GetValueAtIndex(i)? };

            let mut pid_i32: i32 = 0;
            unsafe { info.ProcessId(&mut pid_i32)? };
            let pid = pid_i32 as u32;

            let mut kind = unsafe { core::mem::zeroed::<COREWEBVIEW2_PROCESS_KIND>() };
            unsafe { info.Kind(&mut kind)? };

            if kind == COREWEBVIEW2_PROCESS_KIND_RENDERER {
                governor.attach(pid)?;
                attached.push(pid);
                tracing::info!(pid, "renderer attached to job object");
            }
        }

        Ok(attached)
    }
}

#[cfg(not(windows))]
pub mod win {
    use anyhow::Result;
    use wry::WebView;

    pub struct Governor;

    impl Governor {
        pub fn new(_per_process_bytes: usize, _total_bytes: usize) -> Result<Self> {
            Ok(Self)
        }
        pub fn attach(&self, _pid: u32) -> Result<()> {
            Ok(())
        }
    }

    pub fn enumerate_and_attach_renderers(
        _governor: &Governor,
        _webview: &WebView,
    ) -> Result<Vec<u32>> {
        Ok(vec![])
    }
}