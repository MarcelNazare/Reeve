#[cfg(windows)]
pub mod win {
    use anyhow::Result;
    use win32job::{ExtendedLimitInfo, Job};

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
}