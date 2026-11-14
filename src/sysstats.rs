//! CPU and memory load for the stats faces (Windows only, no extra crates). GPU load is not sampled (reported as 0).

use std::collections::VecDeque;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

const HISTORY: usize = 60;

/// What the stats faces draw: current load in percent plus the last ~minute of samples (oldest first).
#[derive(Clone, Default, Debug)]
pub struct Stats {
    pub cpu: f32,
    pub mem: f32,
    pub hist_cpu: Vec<f32>,
    pub hist_mem: Vec<f32>,
}

fn ticks(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

fn times() -> Option<(u64, u64, u64)> {
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut idle, mut kernel, mut user) = (z, z, z);
    // SAFETY: plain out-parameters of the right type
    let ok = unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) };
    (ok != 0).then(|| (ticks(idle), ticks(kernel), ticks(user)))
}

fn memory_percent() -> f32 {
    // SAFETY: dwLength is set as the API requires
    unsafe {
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) != 0 { m.dwMemoryLoad as f32 } else { 0.0 }
    }
}

pub struct Sampler {
    prev: Option<(u64, u64, u64)>,
    cpu: VecDeque<f32>,
    mem: VecDeque<f32>,
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler { prev: times(), cpu: VecDeque::new(), mem: VecDeque::new() }
    }

    /// Take one sample (call about once a second). CPU is the load since the previous call.
    pub fn sample(&mut self) {
        let now = times();
        let cpu = match (self.prev, now) {
            (Some((i0, k0, u0)), Some((i1, k1, u1))) => {
                // kernel time includes idle time
                let total = (k1 - k0) + (u1 - u0);
                if total == 0 { 0.0 } else { (100.0 * (1.0 - (i1 - i0) as f32 / total as f32)).clamp(0.0, 100.0) }
            }
            _ => 0.0,
        };
        self.prev = now;
        for (q, v) in [(&mut self.cpu, cpu), (&mut self.mem, memory_percent())] {
            q.push_back(v);
            while q.len() > HISTORY {
                q.pop_front();
            }
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            cpu: self.cpu.back().copied().unwrap_or(0.0),
            mem: self.mem.back().copied().unwrap_or(0.0),
            hist_cpu: self.cpu.iter().copied().collect(),
            hist_mem: self.mem.iter().copied().collect(),
        }
    }
}
