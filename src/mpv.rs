//! Small, dynamically loaded subset of libmpv's stable C client API.
use libloading::Library;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    path::Path,
    ptr::NonNull,
};

type Handle = *mut c_void;
type SetString = unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int;

#[repr(C)]
struct RawEvent {
    id: c_int,
    error: c_int,
    userdata: u64,
    data: *mut c_void,
}
#[repr(C)]
struct EndFile {
    reason: c_int,
    error: c_int,
}

pub enum Event {
    None,
    Loaded,
    Ended { eof: bool, failed: bool },
    Shutdown,
}

pub struct Mpv {
    handle: NonNull<c_void>,
    initialize: unsafe extern "C" fn(Handle) -> c_int,
    option: SetString,
    set: SetString,
    get: unsafe extern "C" fn(Handle, *const c_char) -> *mut c_char,
    command: unsafe extern "C" fn(Handle, *const *const c_char) -> c_int,
    wait: unsafe extern "C" fn(Handle, f64) -> *const RawEvent,
    free: unsafe extern "C" fn(*mut c_void),
    destroy: unsafe extern "C" fn(Handle),
    _library: Library,
    #[cfg(windows)]
    _lease: Option<std::fs::File>,
}

impl Mpv {
    pub fn new() -> Result<Self, String> {
        let path = crate::runtime::library_path();
        Self::from_path(&path).or_else(|error| {
            let current = crate::runtime::library_path();
            if current == path {
                Err(error)
            } else {
                Self::from_path(&current)
            }
        })
    }

    pub(crate) fn from_path(path: &Path) -> Result<Self, String> {
        #[cfg(windows)]
        let lease = crate::runtime::lease(path)?;
        // SAFETY: load the packaged library from an explicit path. Every symbol below follows
        // include/mpv/client.h's C ABI; the Library outlives the handle and function pointers.
        unsafe {
            let library = Library::new(path)
                .map_err(|_| "Cannot load the playback libraries. Check the runtime DLL and your graphics drivers.".to_owned())?;
            let create = *library
                .get::<unsafe extern "C" fn() -> Handle>(b"mpv_create\0")
                .map_err(|_| "Invalid libmpv library.")?;
            // Resolve all fallible symbols before allocating the handle.
            let initialize = *library
                .get(b"mpv_initialize\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let option = *library
                .get(b"mpv_set_option_string\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let set = *library
                .get(b"mpv_set_property_string\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let get = *library
                .get(b"mpv_get_property_string\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let command = *library
                .get(b"mpv_command\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let wait = *library
                .get(b"mpv_wait_event\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let free = *library
                .get(b"mpv_free\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let destroy = *library
                .get(b"mpv_terminate_destroy\0")
                .map_err(|_| "Invalid libmpv library.")?;
            let handle = NonNull::new(create()).ok_or("Could not create the playback engine.")?;
            Ok(Self {
                handle,
                initialize,
                option,
                set,
                get,
                command,
                wait,
                free,
                destroy,
                _library: library,
                #[cfg(windows)]
                _lease: lease,
            })
        }
    }

    pub fn option(&self, name: &str, value: &str) -> Result<(), String> {
        self.string_call(self.option, name, value)
    }
    pub fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.string_call(self.set, name, value)
    }

    fn string_call(&self, function: SetString, name: &str, value: &str) -> Result<(), String> {
        let name = cstring(name)?;
        let value = cstring(value)?;
        // SAFETY: the live handle and both NUL-terminated strings remain valid during the call.
        check(unsafe { function(self.handle.as_ptr(), name.as_ptr(), value.as_ptr()) })
    }

    pub fn initialize(&self) -> Result<(), String> {
        // SAFETY: called once on the newly created handle, after options are configured.
        check(unsafe { (self.initialize)(self.handle.as_ptr()) })
    }

    pub fn get(&self, name: &str) -> Option<String> {
        let name = cstring(name).ok()?;
        // SAFETY: libmpv returns a NUL-terminated owned allocation or null. Copy, then free
        // with libmpv's allocator, never Rust's. This worker owns all calls to the handle.
        unsafe {
            let value = (self.get)(self.handle.as_ptr(), name.as_ptr());
            if value.is_null() {
                return None;
            }
            let result = CStr::from_ptr(value).to_string_lossy().into_owned();
            (self.free)(value.cast());
            Some(result)
        }
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let strings = args
            .iter()
            .map(|s| cstring(s))
            .collect::<Result<Vec<_>, _>>()?;
        let pointers = strings
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect::<Vec<_>>();
        // SAFETY: a null-terminated argument array; all CString allocations outlive the call.
        check(unsafe { (self.command)(self.handle.as_ptr(), pointers.as_ptr()) })
    }

    pub fn event(&self, timeout: f64) -> Event {
        // SAFETY: the event and its payload remain valid until the next wait_event call.
        unsafe {
            let Some(event) = (self.wait)(self.handle.as_ptr(), timeout).as_ref() else {
                return Event::None;
            };
            match event.id {
                1 => Event::Shutdown,
                8 => Event::Loaded,
                7 => {
                    let payload = event.data.cast::<EndFile>().as_ref();
                    Event::Ended {
                        eof: payload.is_some_and(|p| p.reason == 0),
                        failed: payload.is_none_or(|p| p.error < 0 || p.reason == 4),
                    }
                }
                _ => Event::None,
            }
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // SAFETY: the worker owns the handle; no callbacks or other threads use it.
        unsafe {
            (self.destroy)(self.handle.as_ptr());
        }
    }
}

fn cstring(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|_| "A playback value contains an invalid NUL character.".into())
}

fn check(code: c_int) -> Result<(), String> {
    // Do not surface libmpv's diagnostic strings: they can contain authenticated URLs/headers.
    if code >= 0 {
        Ok(())
    } else {
        Err(format!(
            "The playback engine rejected a request (code {code})."
        ))
    }
}
