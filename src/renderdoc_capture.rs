#[cfg(all(feature = "debugger_capture", target_os = "linux"))]
mod native {
    use renderdoc_sys::RENDERDOC_API_1_4_1;
    use std::{
        ffi::{CStr, c_void},
        path::PathBuf,
        ptr,
    };

    pub struct Session {
        api: RENDERDOC_API_1_4_1,
        _library: libloading::os::unix::Library,
        before: u32,
        active: bool,
    }

    impl Session {
        pub fn start() -> Result<Self, String> {
            type GetApi = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
            let library = libloading::os::unix::Library::this();
            let mut address = ptr::null_mut();
            // Resolves the injected API and copies the requested version's function table.
            let api = unsafe {
                let get = library
                    .get::<GetApi>(b"RENDERDOC_GetAPI\0")
                    .map_err(|error| {
                        format!("RenderDoc must be injected before launch: {error}")
                    })?;
                if get(10401, &mut address) != 1 || address.is_null() {
                    return Err("RenderDoc API 1.4.1 unavailable".into());
                }
                *address.cast::<RENDERDOC_API_1_4_1>()
            };
            if api.StartFrameCapture.is_none()
                || api.EndFrameCapture.is_none()
                || api.IsFrameCapturing.is_none()
                || api.GetNumCaptures.is_none()
                || api.GetCapture.is_none()
            {
                return Err("RenderDoc capture API incomplete".into());
            }
            // Starts a capture only when the injected debugger is idle.
            unsafe {
                if api.IsFrameCapturing.unwrap()() != 0 {
                    return Err("another RenderDoc capture is active".into());
                }
                let before = api.GetNumCaptures.unwrap()();
                api.StartFrameCapture.unwrap()(ptr::null_mut(), ptr::null_mut());
                if api.IsFrameCapturing.unwrap()() == 0 {
                    return Err("RenderDoc did not start capturing".into());
                }
                Ok(Self {
                    api,
                    _library: library,
                    before,
                    active: true,
                })
            }
        }

        pub fn finish(mut self) -> Result<PathBuf, String> {
            // Ends the owned capture and queries its newly written file.
            unsafe {
                self.active = false;
                if self.api.EndFrameCapture.unwrap()(ptr::null_mut(), ptr::null_mut()) != 1 {
                    return Err("RenderDoc could not save the captured frame".into());
                }
                let count = self.api.GetNumCaptures.unwrap()();
                if self.before.checked_add(1) != Some(count) {
                    return Err("RenderDoc capture count mismatch".into());
                }
                let get = self.api.GetCapture.unwrap();
                let mut length = 0;
                if get(self.before, ptr::null_mut(), &mut length, ptr::null_mut()) != 1
                    || !(1..=8192).contains(&length)
                {
                    return Err("RenderDoc capture path missing or oversized".into());
                }
                let mut path = vec![0u8; length as usize];
                if get(
                    self.before,
                    path.as_mut_ptr().cast(),
                    &mut length,
                    ptr::null_mut(),
                ) != 1
                {
                    return Err("RenderDoc capture path unavailable".into());
                }
                let path = CStr::from_bytes_until_nul(&path)
                    .map_err(|error| format!("invalid RenderDoc path: {error}"))?;
                use std::os::unix::ffi::OsStrExt;
                let path = PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes()));
                if !path.is_file() || path.metadata().map_err(|error| error.to_string())?.len() == 0
                {
                    return Err("RenderDoc capture file is missing or empty".into());
                }
                Ok(path)
            }
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            if self.active {
                // Ends an unfinished owned capture before releasing the API handle.
                unsafe {
                    self.api.EndFrameCapture.unwrap()(ptr::null_mut(), ptr::null_mut());
                }
            }
        }
    }
}

#[cfg(all(feature = "debugger_capture", target_os = "linux"))]
pub use native::Session;

#[cfg(not(all(feature = "debugger_capture", target_os = "linux")))]
pub struct Session;

#[cfg(not(all(feature = "debugger_capture", target_os = "linux")))]
impl Session {
    pub fn start() -> Result<Self, String> {
        Err("RenderDoc requires the Linux debugger_capture build".into())
    }

    pub fn finish(self) -> Result<std::path::PathBuf, String> {
        Err("RenderDoc requires the Linux debugger_capture build".into())
    }
}
