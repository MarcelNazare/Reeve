#[cfg(windows)]
pub mod win {
    use anyhow::Result;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use windows::core::Interface;
    use win32job::{ExtendedLimitInfo, Job};
    use wry::{WebView, WebviewExtWindows};

    pub struct Governor {
        job: Job,
    }

    impl Governor {
        pub fn new(per_process_bytes: usize, total_bytes: usize) -> Result<Self> {
            let mut info = ExtendedLimitInfo::new();
            info.limit_process_memory(per_process_bytes);
            info.limit_job_memory(total_bytes);
            info.limit_kill_on_job_close();
            let job = Job::create_with_limit_info(&mut info)?;
            Ok(Self { job })
        }

        pub fn attach(&self, pid: u32) -> Result<()> {
            self.job.assign_process(pid)?;
            Ok(())
        }
    }

    pub fn enumerate_and_attach_renderers(
        governor: &Governor,
        webview: &WebView,
    ) -> Result<Vec<u32>> {
        let controller = webview.controller();
        let core = unsafe { controller.CoreWebView2()? };
        let env = unsafe { core.Environment()? };

        let env13: ICoreWebView2Environment13 = env.cast()?;

        let collection = unsafe { env13.GetProcessInfos()? };
        let count = unsafe { collection.Count()? };

        let mut attached = Vec::new();

        for i in 0..count {
            let info = unsafe { collection.GetValueAtIndex(i)? };
            let pid = unsafe { info.ProcessId()? };
            let kind = unsafe { info.Kind()? };

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