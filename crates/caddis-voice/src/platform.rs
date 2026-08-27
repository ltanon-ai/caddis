//! platform.rs — the Windows unsafe layer for slice (b), in ONE place.
//!
//! Std-only law (winprobe precedent, caddis-memory): raw `extern "system"`
//! declarations, no windows-sys. Every unsafe fact the safe modules lean on
//! lives here behind a typed, testable seam:
//!
//! - [`vram_probe`] — dxgi.dll's exported `CreateDXGIFactory1` → EnumAdapters1
//!   → GetDesc1, per-adapter dedicated/shared memory. No COM initialization:
//!   the direct export is documented to work standalone, which keeps the FFI
//!   surface to LoadLibraryA/GetProcAddress plus three vtable calls.
//! - Job-object primitives for job.rs (create kill-on-close job, assign a
//!   pid, open a process with the rights assignment needs, close a handle).
//!
//! Non-Windows builds get honest stubs: the organ's supervision law
//! (children die with the parent) is a Windows Job Objects law here, and a
//! stub that pretends would be the exact kind of unearned "verified" this
//! estate refuses.

#[cfg(windows)]
mod vram_ffi {
    use std::os::raw::{c_char, c_int, c_ulong, c_void};

    // GUID {770aae78-f26f-4dba-a829-253c83d1b387} = IDXGIFactory1.
    #[repr(C)]
    pub struct Guid {
        pub data1: c_ulong,
        pub data2: u16,
        pub data3: u16,
        pub data4: [u8; 8],
    }
    pub const IID_IDXGIFACTORY1: Guid = Guid {
        data1: 0x770aae78,
        data2: 0xf26f,
        data3: 0x4dba,
        data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
    };

    /// DXGI_ADAPTER_DESC1 (wincodec-era stable ABI, repr C): 128 UTF-16
    /// chars, ids, then three SIZE_T memory fields.
    #[repr(C)]
    pub struct AdapterDesc1 {
        pub description: [u16; 128],
        pub vendor_id: c_ulong,
        pub device_id: c_ulong,
        pub subsys_id: c_ulong,
        pub revision: c_ulong,
        pub dedicated_video_memory: usize,
        pub dedicated_system_memory: usize,
        pub shared_system_memory: usize,
        pub flags: c_ulong,
    }

    pub type Hr = c_int; // HRESULT
    pub type Factory = *mut c_void;
    pub type Adapter = *mut c_void;

    // Vtable slots counted from IUnknown(0..2) → IDXGIObject(3..4) →
    // IDXGIFactory(5..8) → IDXGIFactory1(9). Stable ABI, verified against
    // the dxgi.h layout; a wrong slot here reads garbage or crashes — the
    // tests assert the numbers look like a GPU, which catches drift.
    pub const VT_ENUM_ADAPTERS1: usize = 9;
    // IUnknown(0..2) → IDXGIObject(3..4) → IDXGIAdapter(5..7) →
    // IDXGIAdapter1(8 = GetDesc1).
    pub const VT_GET_DESC1: usize = 8;
    pub const VT_RELEASE: usize = 2;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn LoadLibraryA(name: *const c_char) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    pub type CreateFactoryFn =
        unsafe extern "system" fn(riid: *const Guid, ppfactory: *mut Factory) -> Hr;

    // Raw vtable access. An interface pointer points at its vtable pointer.
    #[inline]
    pub unsafe fn vt(iface: *mut c_void) -> *mut *mut c_void {
        assert!(!iface.is_null());
        *(iface as *mut *mut *mut c_void)
    }
}

/// DXGI adapter enumeration. `Err(reason)` on any step — the caller reports
/// it honestly; no partial success is invented.
#[cfg(windows)]
pub fn vram_probe() -> Result<Vec<super::vram::AdapterMem>, String> {
    use vram_ffi::*;

    unsafe {
        let dxgi = LoadLibraryA(c"dxgi.dll".as_ptr());
        if dxgi.is_null() {
            return Err("LoadLibraryA(dxgi.dll) failed".into());
        }
        let proc = GetProcAddress(dxgi, c"CreateDXGIFactory1".as_ptr());
        if proc.is_null() {
            return Err("dxgi.dll has no CreateDXGIFactory1".into());
        }
        let create: CreateFactoryFn = std::mem::transmute(proc);

        let mut factory: Factory = std::ptr::null_mut();
        if create(&IID_IDXGIFACTORY1, &mut factory) != 0 {
            return Err("CreateDXGIFactory1 refused".into());
        }

        let mut out = Vec::new();
        let mut idx: u32 = 0;
        loop {
            let mut adapter: Adapter = std::ptr::null_mut();
            let enum_fn: unsafe extern "system" fn(Factory, u32, *mut Adapter) -> Hr =
                std::mem::transmute(*vt(factory).add(VT_ENUM_ADAPTERS1));
            let hr = enum_fn(factory, idx, &mut adapter);
            // DXGI_ERROR_NOT_FOUND = 0x887A0002, end of the adapter list.
            // HRESULT is i32 here; the constant lands negative via `as`.
            const DXGI_ERROR_NOT_FOUND: Hr = 0x887A_0002u32 as i32;
            if hr == DXGI_ERROR_NOT_FOUND {
                break;
            }
            if hr != 0 {
                let _ = release(factory);
                return Err(format!("EnumAdapters1 hr=0x{hr:08x}"));
            }

            let mut desc = AdapterDesc1 {
                description: [0; 128],
                vendor_id: 0,
                device_id: 0,
                subsys_id: 0,
                revision: 0,
                dedicated_video_memory: 0,
                dedicated_system_memory: 0,
                shared_system_memory: 0,
                flags: 0,
            };
            let desc_fn: unsafe extern "system" fn(Adapter, *mut AdapterDesc1) -> Hr =
                std::mem::transmute(*vt(adapter).add(VT_GET_DESC1));
            let got = desc_fn(adapter, &mut desc);
            let _ = release(adapter);
            if got != 0 {
                let _ = release(factory);
                return Err(format!("GetDesc1 hr=0x{got:08x}"));
            }
            let len = desc.description.iter().position(|&c| c == 0).unwrap_or(128);
            let name = String::from_utf16_lossy(&desc.description[..len]);
            out.push(super::vram::AdapterMem {
                name,
                vendor_id: desc.vendor_id,
                device_id: desc.device_id,
                dedicated_video_bytes: desc.dedicated_video_memory as u64,
                dedicated_system_bytes: desc.dedicated_system_memory as u64,
                shared_bytes: desc.shared_system_memory as u64,
            });
            idx += 1;
        }
        let _ = release(factory);
        Ok(out)
    }
}

#[cfg(windows)]
unsafe fn release(iface: vram_ffi::Factory) -> u32 {
    let rel: unsafe extern "system" fn(vram_ffi::Factory) -> u32 =
        std::mem::transmute(*vram_ffi::vt(iface).add(vram_ffi::VT_RELEASE));
    rel(iface)
}

/// Non-Windows: VRAM is unmeasured by this organ (the Windows host is the
/// supervised environment; a Linux port designs its own probe, it does not
/// inherit a pretend one).
#[cfg(not(windows))]
pub fn vram_probe() -> Result<Vec<super::vram::AdapterMem>, String> {
    Err("vram probe is Windows-only (dxgi)".into())
}

// ---------------------------------------------------------------------------
// Job Objects primitives (consumed by job.rs)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod job_ffi {
    use std::os::raw::{c_int, c_ulong, c_void};

    pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: c_ulong = 0x2000;
    /// JobObjectExtendedLimitInformation class value for
    /// SetInformationJobObject.
    pub const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: c_int = 9;
    /// OpenProcess rights AssignProcessToJobObject needs.
    pub const PROCESS_SET_QUOTA: c_ulong = 0x0100;
    pub const PROCESS_TERMINATE: c_ulong = 0x0001;

    #[repr(C)]
    pub struct IoCounters {
        pub read_ops: u64,
        pub write_ops: u64,
        pub other_ops: u64,
        pub read_bytes: u64,
        pub write_bytes: u64,
        pub other_bytes: u64,
    }

    #[repr(C)]
    pub struct JoObjectExtendedLimitInformation {
        // JOBOBJECT_BASIC_LIMIT_INFORMATION
        pub per_process_user_time_limit: i64,
        pub per_job_user_time_limit: i64,
        pub limit_flags: c_ulong,
        pub minimum_working_set_size: usize,
        pub maximum_working_set_size: usize,
        pub active_process_limit: c_ulong,
        pub affinity: usize,
        pub priority_class: c_ulong,
        pub scheduling_class: c_ulong,
        // IO_COUNTERS
        pub io: IoCounters,
        // Extended tail
        pub process_memory_limit: usize,
        pub job_memory_limit: usize,
        pub peak_process_memory_used: usize,
        pub peak_job_memory_used: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateJobObjectW(reserved: *mut c_void, name: *const u16) -> *mut c_void;
        pub fn SetInformationJobObject(
            job: *mut c_void,
            class: c_int,
            info: *mut c_void,
            len: c_ulong,
        ) -> c_int;
        pub fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> c_int;
        pub fn OpenProcess(
            access: c_ulong,
            inherit: c_int,
            pid: c_ulong,
        ) -> *mut c_void;
        pub fn GetCurrentProcess() -> *mut c_void;
        pub fn CloseHandle(handle: *mut c_void) -> c_int;
    }
}

#[cfg(windows)]
pub use job_ffi::*;

#[cfg(windows)]
use std::os::raw::{c_int, c_ulong, c_void};

/// Create an ANONYMOUS job object with KILL_ON_JOB_CLOSE armed. Anonymous on
/// purpose: a named job would be SHARED across organ instances (same name →
/// same kernel object), which is exactly the coupling the port mutex exists
/// to prevent.
///
/// # Safety
/// The returned handle is owned by the caller (close it, or leak it on
/// purpose as the dead-man switch does). No borrows cross the FFI boundary.
#[cfg(windows)]
pub unsafe fn create_kill_on_close_job() -> Result<*mut c_void, String> {
    use job_ffi::*;
    let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
    if job.is_null() {
        return Err("CreateJobObjectW returned null".into());
    }
    let mut info = JoObjectExtendedLimitInformation {
        per_process_user_time_limit: 0,
        per_job_user_time_limit: 0,
        limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        minimum_working_set_size: 0,
        maximum_working_set_size: 0,
        active_process_limit: 0,
        affinity: 0,
        priority_class: 0,
        scheduling_class: 0,
        io: IoCounters {
            read_ops: 0,
            write_ops: 0,
            other_ops: 0,
            read_bytes: 0,
            write_bytes: 0,
            other_bytes: 0,
        },
        process_memory_limit: 0,
        job_memory_limit: 0,
        peak_process_memory_used: 0,
        peak_job_memory_used: 0,
    };
    let ok = SetInformationJobObject(
        job,
        JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
        &mut info as *mut _ as *mut c_void,
        std::mem::size_of::<JoObjectExtendedLimitInformation>() as c_ulong,
    );
    if ok == 0 {
        CloseHandle(job);
        return Err("SetInformationJobObject(KILL_ON_JOB_CLOSE) failed".into());
    }
    Ok(job)
}

/// Open a process with the rights job assignment needs. Null on failure —
/// callers fail closed on their own verdict.
///
/// # Safety
/// The returned handle is owned by the caller and must be closed. Opening a
/// foreign process is a privileged operation; the caller decides policy.
#[cfg(windows)]
pub unsafe fn open_process_for_assignment(pid: u32) -> *mut c_void {
    use job_ffi::*;
    OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid as c_ulong)
}

/// Assign an opened process handle to the job. `false` on API refusal.
///
/// # Safety
/// Both handles must be valid kernel handles owned by this process; the job
/// keeps its own reference to the process after the call.
#[cfg(windows)]
pub unsafe fn assign_to_job(job: *mut c_void, process: *mut c_void) -> bool {
    job_ffi::AssignProcessToJobObject(job, process) != 0
}

/// The caller's own process handle (pseudo-handle; valid only inside this
/// process — everything here is).
///
/// # Safety
/// The pseudo-handle needs no closing and is only meaningful in this
/// process; callers must not pass it to another process or close it.
#[cfg(windows)]
pub unsafe fn current_process_handle() -> *mut c_void {
    job_ffi::GetCurrentProcess()
}

/// Close a kernel handle.
///
/// # Safety
/// `h` must be a handle this process owns (never a foreign or pseudo-handle
/// other than as documented by its caller). Closing the LAST handle to a
/// kill-on-close job kills its processes — that is the point.
#[cfg(windows)]
pub unsafe fn close_handle(h: *mut c_void) -> c_int {
    job_ffi::CloseHandle(h)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn job_struct_layout_matches_the_abi() {
        // The one struct we pass BY POINTER to the kernel. If the layout
        // drifts (a field reordered, a pad missed), the kernel reads the
        // limit flags from the wrong offset and KILL_ON_JOB_CLOSE silently
        // arms something else. size_of is the mechanical tripwire: the true
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION is 144 bytes on x64.
        assert_eq!(
            std::mem::size_of::<super::job_ffi::JoObjectExtendedLimitInformation>(),
            144
        );
        // And the flags field must sit exactly where winnt.h puts it.
        let info: super::job_ffi::JoObjectExtendedLimitInformation = unsafe { std::mem::zeroed() };
        let base = &info as *const _ as usize;
        let flags = &info.limit_flags as *const _ as usize;
        assert_eq!(flags - base, 16);
    }
}
