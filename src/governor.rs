#[cfg(windows)]
pub mod win {
    use anyhow::{anyhow, Result};
    use serde::Serialize;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::GetProcessExtendedInfosCompletedHandler;
    use windows::core::{Interface, PWSTR, BOOL};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
        PROCESS_TERMINATE,
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

    #[derive(Debug, Clone, Copy, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ProcessKind {
        Browser,
        Renderer,
        Gpu,
        Utility,
        Other,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct ProcessSnapshot {
        pub pid: u32,
        pub kind: ProcessKind,
        pub memory_bytes: Option<u64>,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct AppMemoryUsage {
        pub app_id: String,
        pub memory_bytes: u64,
    }

    pub fn query_process_memory(pid: u32) -> Option<u64> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            if handle.is_invalid() {
                return None;
            }

            let mut counters = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            let result = GetProcessMemoryInfo(handle, &mut counters, counters.cb);
            let _ = CloseHandle(handle);

            if result.is_err() {
                return None;
            }
            Some(counters.WorkingSetSize as u64)
        }
    }

    pub fn snapshot_processes(webview: &WebView) -> Result<Vec<ProcessSnapshot>> {
        let controller = webview.controller();
        let core = unsafe { controller.CoreWebView2()? };
        let core2: ICoreWebView2_2 = core.cast()?;
        let env = unsafe { core2.Environment()? };
        let env13: ICoreWebView2Environment13 = env.cast()?;
        let collection = unsafe { env13.GetProcessInfos()? };

        let mut count: u32 = 0;
        unsafe { collection.Count(&mut count)? };

        let mut result = Vec::new();
        for i in 0..count {
            let info = match unsafe { collection.GetValueAtIndex(i) } {
                Ok(i) => i,
                Err(_) => continue,
            };

            let mut pid_i32: i32 = 0;
            if unsafe { info.ProcessId(&mut pid_i32) }.is_err() {
                continue;
            }
            let pid = pid_i32 as u32;

            let mut raw_kind = unsafe { core::mem::zeroed::<COREWEBVIEW2_PROCESS_KIND>() };
            if unsafe { info.Kind(&mut raw_kind) }.is_err() {
                continue;
            }

            let kind = match raw_kind {
                COREWEBVIEW2_PROCESS_KIND_BROWSER => ProcessKind::Browser,
                COREWEBVIEW2_PROCESS_KIND_RENDERER => ProcessKind::Renderer,
                COREWEBVIEW2_PROCESS_KIND_GPU => ProcessKind::Gpu,
                COREWEBVIEW2_PROCESS_KIND_UTILITY => ProcessKind::Utility,
                _ => ProcessKind::Other,
            };

            result.push(ProcessSnapshot {
                pid,
                kind,
                memory_bytes: query_process_memory(pid),
            });
        }

        Ok(result)
    }

    pub fn query_app_memory_async(
        webview: &WebView,
        app_urls: Vec<(String, String)>,
        callback: Box<dyn FnOnce(Vec<AppMemoryUsage>) + Send>,
    ) -> Result<()> {
        let controller = webview.controller();
        let core = unsafe { controller.CoreWebView2()? };
        let core2: ICoreWebView2_2 = core.cast()?;
        let env = unsafe { core2.Environment()? };
        let env13: ICoreWebView2Environment13 = env.cast()?;

        let handler = GetProcessExtendedInfosCompletedHandler::create(Box::new(
            move |error, collection| -> windows::core::Result<()> {
                let mut usage = Vec::new();

                if error.is_err() {
                    callback(usage);
                    return Ok(());
                }

                let collection = match collection {
                    Some(c) => c,
                    None => {
                        callback(usage);
                        return Ok(());
                    }
                };

                let mut count = 0;
                if unsafe { collection.Count(&mut count) }.is_err() {
                    callback(usage);
                    return Ok(());
                }

                for i in 0..count {
                    let info = match unsafe { collection.GetValueAtIndex(i) } {
                        Ok(i) => i,
                        Err(_) => continue,
                    };
                    let process_info = match unsafe { info.ProcessInfo() } {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    let mut kind = unsafe { core::mem::zeroed::<COREWEBVIEW2_PROCESS_KIND>() };
                    if unsafe { process_info.Kind(&mut kind) }.is_err() {
                        continue;
                    }
                    if kind != COREWEBVIEW2_PROCESS_KIND_RENDERER {
                        continue;
                    }

                    let mut pid_i32 = 0;
                    if unsafe { process_info.ProcessId(&mut pid_i32) }.is_err() {
                        continue;
                    }
                    let mem = match query_process_memory(pid_i32 as u32) {
                        Some(m) => m,
                        None => continue,
                    };

                    let frames = match unsafe { info.AssociatedFrameInfos() } {
                        Ok(f) => f,
                        Err(_) => continue,
                    };

                    // ICoreWebView2FrameInfoCollection uses an iterator,
                    // not Count/GetValueAtIndex.
                    let iterator = match unsafe { frames.GetIterator() } {
                        Ok(it) => it,
                        Err(_) => continue,
                    };

                    let mut has_current = BOOL(0);
                    if unsafe { iterator.HasCurrent(&mut has_current) }.is_err() {
                        continue;
                    }

                    while has_current.as_bool() {
                        let frame = match unsafe { iterator.GetCurrent() } {
                            Ok(f) => f,
                            Err(_) => break,
                        };

                        let mut source = PWSTR::null();
                        if unsafe { frame.Source(&mut source) }.is_err() {
                            if unsafe { iterator.MoveNext(&mut has_current) }.is_err() {
                                break;
                            }
                            continue;
                        }

                        let url = match unsafe { source.to_string() } {
                            Ok(u) => u,
                            Err(_) => {
                                if unsafe { iterator.MoveNext(&mut has_current) }.is_err() {
                                    break;
                                }
                                continue;
                            }
                        };

                        if let Some((app_id, _)) = app_urls.iter().find(|(_, app_url)| {
                            url.starts_with(app_url) || app_url.starts_with(&url)
                        }) {
                            usage.push(AppMemoryUsage {
                                app_id: app_id.clone(),
                                memory_bytes: mem,
                            });
                            break; // Done with this process
                        }

                        if unsafe { iterator.MoveNext(&mut has_current) }.is_err() {
                            break;
                        }
                    }
                }

                callback(usage);
                Ok(())
            },
        ));

        unsafe { env13.GetProcessExtendedInfos(&handler)? };
        Ok(())
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
            let info = match unsafe { collection.GetValueAtIndex(i) } {
                Ok(info) => info,
                Err(_) => continue,
            };

            let mut pid_i32: i32 = 0;
            if unsafe { info.ProcessId(&mut pid_i32) }.is_err() {
                continue;
            }
            let pid = pid_i32 as u32;

            let mut kind = unsafe { core::mem::zeroed::<COREWEBVIEW2_PROCESS_KIND>() };
            if unsafe { info.Kind(&mut kind) }.is_err() {
                continue;
            }

            let kind_label: &str = match kind {
                COREWEBVIEW2_PROCESS_KIND_BROWSER => "browser",
                COREWEBVIEW2_PROCESS_KIND_RENDERER => "renderer",
                COREWEBVIEW2_PROCESS_KIND_GPU => "gpu",
                COREWEBVIEW2_PROCESS_KIND_UTILITY => "utility",
                _ => "other",
            };

            match governor.attach(pid) {
                Ok(()) => {
                    attached.push(pid);
                    tracing::info!(pid, kind = kind_label, "process attached to job object");
                }
                Err(e) => {
                    tracing::debug!(pid, kind = kind_label, error = %e, "process not attached");
                }
            }
        }

        Ok(attached)
    }
}

#[cfg(not(windows))]
pub mod win {
    use anyhow::Result;
    use serde::Serialize;
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

    #[derive(Debug, Clone, Copy, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ProcessKind {
        Browser,
        Renderer,
        Gpu,
        Utility,
        Other,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct ProcessSnapshot {
        pub pid: u32,
        pub kind: ProcessKind,
        pub memory_bytes: Option<u64>,
    }

    #[derive(Debug, Clone, Serialize)]
    pub struct AppMemoryUsage {
        pub app_id: String,
        pub memory_bytes: u64,
    }

    pub fn snapshot_processes(_webview: &WebView) -> Result<Vec<ProcessSnapshot>> {
        Ok(vec![])
    }

    pub fn query_app_memory_async(
        _webview: &WebView,
        _app_urls: Vec<(String, String)>,
        callback: Box<dyn FnOnce(Vec<AppMemoryUsage>) + Send>,
    ) -> Result<()> {
        callback(vec![]);
        Ok(())
    }

    pub fn enumerate_and_attach_renderers(
        _governor: &Governor,
        _webview: &WebView,
    ) -> Result<Vec<u32>> {
        Ok(vec![])
    }
}