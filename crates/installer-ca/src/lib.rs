//! MSI custom action that fills AVAILABLE_SPACE for the destination drive.

use std::ptr;

#[link(name = "msi")]
extern "system" {
    fn MsiGetPropertyW(
        install: u32,
        name: *const u16,
        buffer: *mut u16,
        length: *mut u32,
    ) -> u32;

    fn MsiGetTargetPathW(
        install: u32,
        folder: *const u16,
        buffer: *mut u16,
        length: *mut u32,
    ) -> u32;

    fn MsiSetPropertyW(install: u32, name: *const u16, value: *const u16) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetDiskFreeSpaceExW(
        directory: *const u16,
        free_to_caller: *mut u64,
        total: *mut u64,
        total_free: *mut u64,
    ) -> i32;
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn property(install: u32, name: &str) -> Option<String> {
    let name = wide(name);
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;

    let result = unsafe {
        MsiGetPropertyW(install, name.as_ptr(), buffer.as_mut_ptr(), &mut length)
    };

    if result != 0 || length == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

fn set_property(install: u32, name: &str, value: &str) {
    let name = wide(name);
    let value = wide(value);

    unsafe {
        MsiSetPropertyW(install, name.as_ptr(), value.as_ptr());
    }
}

fn target_path(install: u32) -> Option<String> {
    let folder = wide("INSTALLFOLDER");
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;

    let result = unsafe {
        MsiGetTargetPathW(install, folder.as_ptr(), buffer.as_mut_ptr(), &mut length)
    };

    if result != 0 || length == 0 {
        return None;
    }

    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

#[no_mangle]
pub extern "system" fn SetDiskSpace(install: u32) -> u32 {
    let path = target_path(install)
        .or_else(|| property(install, "INSTALLFOLDER"))
        .unwrap_or_default();

    if path.is_empty() {
        set_property(install, "AVAILABLE_SPACE", "unknown");
        return 0;
    }

    let path = wide(&path);
    let mut free = 0u64;

    let ok = unsafe {
        GetDiskFreeSpaceExW(path.as_ptr(), &mut free, ptr::null_mut(), ptr::null_mut())
    };

    if ok != 0 {
        let gigabytes = free as f64 / 1_073_741_824.0;
        set_property(install, "AVAILABLE_SPACE", &format!("{gigabytes:.1} GB"));
    } else {
        set_property(install, "AVAILABLE_SPACE", "unknown");
    }

    0
}
