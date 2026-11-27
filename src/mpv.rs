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
    StartFile(i64),
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
    _directory: Option<DllDirectory>,
    #[cfg(windows)]
    _lease: Option<std::fs::File>,
}

/// Limits later DLL loads in this process to the application folder, System32 and folders
/// added with AddDllDirectory, so neither the working directory nor PATH supplies a DLL.
/// Call before other threads start or load libraries.
pub fn harden_dll_search() {
    #[cfg(windows)]
    // SAFETY: a process-wide loader setting with a documented constant flag.
    unsafe {
        use windows_sys::Win32::System::LibraryLoader::{
            LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, SetDefaultDllDirectories,
        };
        SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    }
}

/// Keeps the runtime folder on the process DLL search list while libmpv is loaded, so helpers
/// it loads by name later (such as the bundled d3dcompiler_43.dll fallback) resolve there.
#[cfg(windows)]
struct DllDirectory(*mut c_void);

#[cfg(windows)]
impl Drop for DllDirectory {
    fn drop(&mut self) {
        // SAFETY: the cookie came from AddDllDirectory and is removed exactly once.
        unsafe {
            windows_sys::Win32::System::LibraryLoader::RemoveDllDirectory(self.0);
        }
    }
}

/// Loads libmpv so its imports resolve from its own folder and System32, never the working
/// directory or PATH. A bare file name searches only the application folder and System32.
#[cfg(windows)]
unsafe fn load(path: &Path) -> Option<(Library, Option<DllDirectory>)> {
    use libloading::os::windows::{
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, Library as Native,
    };
    use std::os::windows::ffi::OsStrExt;
    if path.parent().is_none_or(|dir| dir.as_os_str().is_empty()) {
        // SAFETY: the caller upholds libloading's initialisation requirements.
        let library = unsafe { Native::load_with_flags(path, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) };
        return library.ok().map(|library| (library.into(), None));
    }
    // LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR requires a fully qualified path with backslashes.
    let path = std::path::absolute(path).ok()?;
    let directory = path.parent().and_then(|dir| {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: a NUL-terminated absolute folder path; DllDirectory removes the cookie once.
        let cookie =
            unsafe { windows_sys::Win32::System::LibraryLoader::AddDllDirectory(wide.as_ptr()) };
        (!cookie.is_null()).then_some(DllDirectory(cookie))
    });
    // SAFETY: the caller upholds libloading's initialisation requirements.
    let library = unsafe {
        Native::load_with_flags(
            &path,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
    };
    library.ok().map(|library| (library.into(), directory))
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
            #[cfg(windows)]
            let (library, directory) = load(path)
                .ok_or_else(|| "Cannot load the playback libraries. Check the runtime DLL and your graphics drivers.".to_owned())?;
            #[cfg(not(windows))]
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
                _directory: directory,
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
                // mpv_event_start_file has a single int64_t playlist_entry_id.
                6 => event
                    .data
                    .cast::<i64>()
                    .as_ref()
                    .copied()
                    .map_or(Event::None, Event::StartFile),
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
