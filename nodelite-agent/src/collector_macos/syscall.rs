//! Thin wrappers around macOS libc, Mach, and sysctl calls.

use std::ffi::CStr;
use std::mem::{self, MaybeUninit};
use std::ptr;
use std::slice;

use anyhow::{Context, Result, anyhow};

use super::super::shared::CpuSample;

#[repr(C)]
pub(super) struct IfMibData {
    pub(super) ifmd_name: [libc::c_char; libc::IFNAMSIZ],
    pub(super) ifmd_pcount: libc::c_uint,
    pub(super) ifmd_flags: libc::c_uint,
    pub(super) ifmd_snd_len: libc::c_uint,
    pub(super) ifmd_snd_maxlen: libc::c_uint,
    pub(super) ifmd_snd_drops: libc::c_uint,
    pub(super) ifmd_filler: [libc::c_uint; 4],
    pub(super) ifmd_data: libc::if_data64,
}

pub(super) struct MemoryStatistics {
    pub(super) stats: libc::vm_statistics64,
    pub(super) page_size: u64,
    pub(super) total_bytes: u64,
}

pub(super) struct IfAddrsGuard(*mut libc::ifaddrs);

const IFDATA_GENERAL: libc::c_int = 1;
const IFMIB_IFDATA: libc::c_int = 2;
const NETLINK_GENERIC: libc::c_int = 0;

pub(super) fn read_uname() -> Result<libc::utsname> {
    let mut uts = unsafe { mem::zeroed::<libc::utsname>() };
    let result = unsafe { libc::uname(&mut uts) };
    if result != 0 {
        return Err(anyhow!("uname failed"));
    }
    Ok(uts)
}

pub(super) fn c_chars_to_string(value: &[libc::c_char]) -> Result<String> {
    let text = unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_str()
        .context("invalid utf-8 in C string")?;
    Ok(text.to_string())
}

pub(super) fn read_sysctl_string(name: &[u8]) -> Result<String> {
    let mut size = 0_usize;
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            ptr::null_mut(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size == 0 {
        return Err(anyhow!("sysctlbyname failed"));
    }

    let mut buffer = vec![0_u8; size];
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(anyhow!("sysctlbyname read failed"));
    }
    if matches!(buffer.last(), Some(0)) {
        buffer.pop();
    }
    String::from_utf8(buffer).context("sysctl returned non-utf8 string")
}

pub(super) fn count_cpu_cores() -> Result<u32> {
    let cores = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if cores <= 0 {
        return Err(anyhow!("_SC_NPROCESSORS_ONLN returned {cores}"));
    }
    Ok(u32::try_from(cores).unwrap_or(u32::MAX).max(1))
}

pub(super) fn read_uptime_secs() -> Result<u64> {
    for clock_id in [libc::CLOCK_UPTIME_RAW, libc::CLOCK_MONOTONIC_RAW] {
        let mut spec = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe { libc::clock_gettime(clock_id, &mut spec) };
        if result == 0 {
            return u64::try_from(spec.tv_sec).context("negative uptime reported by clock_gettime");
        }
    }
    Err(anyhow!("clock_gettime failed for uptime clocks"))
}

/// 使用 `host_processor_info` 汇总所有逻辑核心的累计 tick。
pub(super) fn collect_cpu_sample() -> Result<CpuSample> {
    #[allow(deprecated)]
    let host = unsafe { libc::mach_host_self() };
    let mut cpu_count: libc::natural_t = 0;
    let mut cpu_info: libc::processor_cpu_load_info_t = ptr::null_mut();
    let mut info_count: libc::mach_msg_type_number_t = 0;

    let status = unsafe {
        libc::host_processor_info(
            host,
            libc::PROCESSOR_CPU_LOAD_INFO,
            &mut cpu_count,
            (&mut cpu_info as *mut libc::processor_cpu_load_info_t).cast(),
            &mut info_count,
        )
    };
    if status != libc::KERN_SUCCESS || cpu_count == 0 || cpu_info.is_null() {
        return Err(anyhow!("host_processor_info failed"));
    }

    let samples = unsafe { slice::from_raw_parts(cpu_info, cpu_count as usize) };
    let mut total = 0_u64;
    let mut idle = 0_u64;
    for sample in samples {
        for tick in sample.cpu_ticks {
            total = total.saturating_add(u64::from(tick));
        }
        idle = idle.saturating_add(u64::from(sample.cpu_ticks[libc::CPU_STATE_IDLE as usize]));
    }

    let deallocate_size = mem::size_of::<libc::integer_t>().saturating_mul(info_count as usize);
    unsafe {
        libc::vm_deallocate(
            #[allow(deprecated)]
            libc::mach_task_self(),
            cpu_info as _,
            deallocate_size,
        );
    }

    Ok(CpuSample { total, idle })
}

pub(super) fn collect_load_average() -> Result<nodelite_proto::LoadAverage> {
    let mut loads = [0_f64; 3];
    let result = unsafe { libc::getloadavg(loads.as_mut_ptr(), 3) };
    if result != 3 {
        return Err(anyhow!("getloadavg returned {result}"));
    }
    Ok(nodelite_proto::LoadAverage {
        one: loads[0],
        five: loads[1],
        fifteen: loads[2],
    })
}

pub(super) fn read_memory_statistics() -> Result<MemoryStatistics> {
    let total_pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if total_pages <= 0 || page_size <= 0 {
        return Err(anyhow!(
            "sysconf returned invalid memory sizes: pages={total_pages} page_size={page_size}"
        ));
    }

    #[allow(deprecated)]
    let host = unsafe { libc::mach_host_self() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let mut stats = unsafe { mem::zeroed::<libc::vm_statistics64>() };
    let status = unsafe {
        libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&mut stats as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    };
    if status != libc::KERN_SUCCESS {
        return Err(anyhow!("host_statistics64 failed"));
    }

    let total_bytes = u64::try_from(total_pages)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(page_size).unwrap_or(u64::MAX));
    Ok(MemoryStatistics {
        stats,
        page_size: u64::try_from(page_size).unwrap_or(u64::MAX),
        total_bytes,
    })
}

pub(super) fn read_swap_usage() -> Result<(u64, u64)> {
    let mut mib = [libc::CTL_VM, libc::VM_SWAPUSAGE];
    let mut swap = unsafe { mem::zeroed::<libc::xsw_usage>() };
    let mut size = mem::size_of::<libc::xsw_usage>();
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as _,
            (&mut swap as *mut libc::xsw_usage).cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(anyhow!("sysctl VM_SWAPUSAGE failed"));
    }
    Ok((swap.xsu_total, swap.xsu_used))
}

pub(super) fn mounted_filesystems() -> Result<Vec<libc::statfs>> {
    let mut mounts: *mut libc::statfs = ptr::null_mut();
    let count = unsafe { libc::getmntinfo(&mut mounts, libc::MNT_NOWAIT) };
    if count <= 0 || mounts.is_null() {
        return Err(anyhow!("getmntinfo returned no mounts"));
    }

    let mounts = unsafe { slice::from_raw_parts(mounts, count as usize) };
    Ok(mounts.to_vec())
}

pub(super) fn network_iflist2_len() -> Result<usize> {
    let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
    let mut len = 0_usize;
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as _,
            ptr::null_mut(),
            &mut len,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 || len == 0 {
        return Err(anyhow!("sysctl NET_RT_IFLIST2 size query failed"));
    }
    Ok(len)
}

pub(super) fn read_network_iflist2(mut len: usize) -> Result<Vec<u8>> {
    let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
    let mut buffer = vec![0_u8; len];
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as _,
            buffer.as_mut_ptr().cast(),
            &mut len,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(anyhow!("sysctl NET_RT_IFLIST2 read failed"));
    }
    buffer.truncate(len);
    Ok(buffer)
}

pub(super) fn collect_interface_data(index: u16) -> Result<IfMibData> {
    let mut mib_data = [
        libc::CTL_NET,
        libc::PF_LINK,
        NETLINK_GENERIC,
        IFMIB_IFDATA,
        index as libc::c_int,
        IFDATA_GENERAL,
    ];
    let mut if_data = MaybeUninit::<IfMibData>::uninit();
    let mut size = mem::size_of::<IfMibData>();
    let result = unsafe {
        libc::sysctl(
            mib_data.as_mut_ptr(),
            mib_data.len() as _,
            if_data.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };

    if result != 0 || size < mem::size_of::<IfMibData>() {
        return Err(anyhow!("sysctl IFMIB_IFDATA failed for interface {index}"));
    }
    Ok(unsafe { if_data.assume_init() })
}

pub(super) fn get_ifaddrs() -> Result<IfAddrsGuard> {
    let mut addrs: *mut libc::ifaddrs = ptr::null_mut();
    let result = unsafe { libc::getifaddrs(&mut addrs) };
    if result != 0 || addrs.is_null() {
        return Err(anyhow!("getifaddrs failed"));
    }
    Ok(IfAddrsGuard(addrs))
}

impl IfAddrsGuard {
    pub(super) fn as_ptr(&self) -> *mut libc::ifaddrs {
        self.0
    }
}

impl Drop for IfAddrsGuard {
    fn drop(&mut self) {
        unsafe {
            libc::freeifaddrs(self.0);
        }
    }
}
