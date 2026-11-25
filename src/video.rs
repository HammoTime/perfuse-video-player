//! The video surface is a child of Perfuse's native window, never a second process/window.

#[derive(Clone, Copy, Default)]
pub struct OverlayState {
    pub ready: bool,
    pub focused: bool,
    pub hovered: bool,
    pub topmost: bool,
}

#[cfg(windows)]
pub struct Surface(windows_sys::Win32::Foundation::HWND);

#[cfg(windows)]
fn enable_overlay_alpha(window: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::Graphics::{Dwm::*, Gdi::*};
    // eframe 0.33/glutin-winit can clear the transparent flag on secondary
    // windows when a Windows GL driver reports no transparency support:
    // https://github.com/emilk/egui/pull/8423
    // Restore the same DWM setup winit uses for transparent window creation.
    // SAFETY: the caller validated this process's live top-level HWND. The region
    // is owned here and can be freed as soon as DWM has copied its settings.
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        if region.is_null() {
            return false;
        }
        let settings = DWM_BLURBEHIND {
            dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION,
            fEnable: 1,
            hRgnBlur: region,
            fTransitionOnMaximized: 0,
        };
        let result = DwmEnableBlurBehindWindow(window, &settings);
        DeleteObject(region);
        result >= 0
    }
}

#[cfg(windows)]
impl Surface {
    pub fn new(frame: &eframe::Frame) -> Result<Self, String> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::{
            System::LibraryLoader::GetModuleHandleW, UI::WindowsAndMessaging::*,
        };
        let RawWindowHandle::Win32(parent) = frame
            .window_handle()
            .map_err(|_| "Cannot access the app window.")?
            .as_raw()
        else {
            return Err("A native Windows window is required for playback.".into());
        };
        // SAFETY: STATIC is a built-in window class; the parent handle belongs to eframe.
        // The surface is created, positioned, and destroyed on the UI thread.
        let handle = unsafe {
            CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!(""),
                WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                parent.hwnd.get() as _,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err("Could not create the embedded video surface.".into());
        }
        Ok(Self(handle))
    }

    pub fn id(&self) -> u64 {
        self.0 as usize as u32 as u64
    }

    pub fn pointer_position(&self) -> Option<(i32, i32)> {
        let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        // SAFETY: point is writable for the duration of this UI-thread call.
        let found =
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) };
        (found != 0).then_some((point.x, point.y))
    }

    /// Clicking the video can give libmpv's window inside this surface the
    /// keyboard focus, where it would swallow playback shortcuts such as Space.
    /// The cross-thread child shares the app's input queue, so GetFocus sees it.
    pub fn reclaim_keyboard(&self) {
        use windows_sys::Win32::UI::{
            Input::KeyboardAndMouse::{GetFocus, SetFocus},
            WindowsAndMessaging::{GetParent, IsChild},
        };
        // SAFETY: handles come from this surface and its parent; called on the UI thread.
        unsafe {
            let focus = GetFocus();
            if !focus.is_null() && (focus == self.0 || IsChild(self.0, focus) != 0) {
                SetFocus(GetParent(self.0));
            }
        }
    }

    /// Attach the transparent controls viewport to this app, not the desktop.
    /// Mirrors the parent window's always-on-top state without ever making the
    /// overlay globally topmost on its own.
    pub fn prepare_overlay(&self, title: &str) -> OverlayState {
        use windows_sys::Win32::{
            Foundation::{POINT, RECT},
            UI::WindowsAndMessaging::*,
        };
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        // SAFETY: all handles are checked before use. Only an overlay in this
        // process is modified; both windows live on eframe's UI thread.
        unsafe {
            let overlay = FindWindowW(std::ptr::null(), title.as_ptr());
            if overlay.is_null() {
                return OverlayState::default();
            }
            let mut process = 0;
            GetWindowThreadProcessId(overlay, &mut process);
            if process != std::process::id() {
                return OverlayState::default();
            }
            let parent = GetParent(self.0);
            if parent.is_null() {
                return OverlayState::default();
            }
            if GetWindowLongPtrW(overlay, GWLP_HWNDPARENT) != parent as isize {
                // Configure alpha before showing or marking this overlay ready.
                if !enable_overlay_alpha(overlay) {
                    return OverlayState::default();
                }
                SetWindowLongPtrW(overlay, GWLP_HWNDPARENT, parent as isize);
            }
            if GetWindowLongPtrW(overlay, GWLP_HWNDPARENT) != parent as isize {
                return OverlayState::default();
            }
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(self.0, &mut rect) == 0 {
                return OverlayState::default();
            }
            let parent_topmost =
                (GetWindowLongPtrW(parent, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST) != 0;
            let overlay_topmost =
                (GetWindowLongPtrW(overlay, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST) != 0;
            let (insert_after, flags) = if parent_topmost != overlay_topmost {
                (
                    if parent_topmost {
                        HWND_TOPMOST
                    } else {
                        HWND_NOTOPMOST
                    },
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                )
            } else {
                (
                    std::ptr::null_mut(),
                    SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER,
                )
            };
            SetWindowPos(
                overlay,
                insert_after,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                flags,
            );
            let not_minimized = IsIconic(parent) == 0;
            let foreground = GetForegroundWindow();
            let focused = not_minimized && (foreground == parent || foreground == overlay);
            let mut cursor = POINT { x: 0, y: 0 };
            let hovered = not_minimized && GetCursorPos(&mut cursor) != 0 && {
                let hit = WindowFromPoint(cursor);
                if hit.is_null() {
                    false
                } else {
                    let root = GetAncestor(hit, GA_ROOT);
                    root == parent || root == overlay
                }
            };
            OverlayState {
                ready: not_minimized && (focused || parent_topmost),
                focused,
                hovered,
                topmost: parent_topmost,
            }
        }
    }

    pub fn place(&self, rect: egui_rect::Rect, scale: f32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        // SAFETY: the live child handle is owned by this UI-thread surface.
        unsafe {
            SetWindowPos(
                self.0,
                std::ptr::null_mut(),
                (rect.min.x * scale).round() as i32,
                (rect.min.y * scale).round() as i32,
                (rect.width() * scale).round().max(1.0) as i32,
                (rect.height() * scale).round().max(1.0) as i32,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW,
            );
        }
    }
}

#[cfg(windows)]
impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: playback is stopped before the owning UI drops the surface.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0);
        }
    }
}

// The current product targets Windows; other targets can still run API/unit tests.
#[cfg(not(windows))]
pub struct Surface;
#[cfg(not(windows))]
impl Surface {
    pub fn new(_: &eframe::Frame) -> Result<Self, String> {
        Err("Embedded video currently requires Windows.".into())
    }
    pub fn id(&self) -> u64 {
        0
    }
    pub fn pointer_position(&self) -> Option<(i32, i32)> {
        None
    }
    pub fn prepare_overlay(&self, _: &str) -> OverlayState {
        OverlayState::default()
    }
    pub fn reclaim_keyboard(&self) {}
    pub fn place(&self, _: egui_rect::Rect, _: f32) {}
}

use eframe::egui as egui_rect;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows_sys::Win32::{Foundation::RECT, UI::WindowsAndMessaging::*};

    #[test]
    fn transparent_overlay_is_owned_and_tracks_the_video() {
        let title = format!("Perfuse overlay ownership test {}", std::process::id());
        let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        // SAFETY: create only hidden synthetic windows on this test thread. Surface
        // guards destroy them in reverse order, children before their parent.
        unsafe {
            let parent = Surface(CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!(""),
                WS_POPUP,
                100,
                100,
                860,
                620,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ));
            assert!(!parent.0.is_null());
            let overlay = Surface(CreateWindowExW(
                WS_EX_TOOLWINDOW,
                windows_sys::w!("STATIC"),
                wide.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ));
            assert!(!overlay.0.is_null());
            // Start from an ordinary opaque native window, as the affected
            // glutin path does, and require real DWM alpha setup to succeed.
            assert!(enable_overlay_alpha(overlay.0));
            let surface = Surface(CreateWindowExW(
                0,
                windows_sys::w!("STATIC"),
                windows_sys::w!(""),
                WS_CHILD,
                0,
                48,
                860,
                572,
                parent.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            ));
            assert!(!surface.0.is_null());
            assert!(
                !surface
                    .prepare_overlay("Perfuse overlay that does not exist")
                    .ready
            );
            for (x, y) in [(100, 100), (300, 240)] {
                SetWindowPos(
                    parent.0,
                    std::ptr::null_mut(),
                    x,
                    y,
                    860,
                    620,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                let state = surface.prepare_overlay(&title);
                assert!(!state.topmost);
                assert_eq!(
                    GetWindowLongPtrW(overlay.0, GWLP_HWNDPARENT),
                    parent.0 as isize
                );
                assert_eq!(
                    GetWindowLongPtrW(overlay.0, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                    0
                );
                let mut video = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                let mut controls = video;
                assert_ne!(GetWindowRect(surface.0, &mut video), 0);
                assert_ne!(GetWindowRect(overlay.0, &mut controls), 0);
                assert_eq!(
                    (video.left, video.top, video.right, video.bottom),
                    (controls.left, controls.top, controls.right, controls.bottom)
                );
                assert_eq!(IsWindowVisible(overlay.0), 0);
            }
            // Topmost mirrors the parent window and clears again when disabled.
            SetWindowPos(
                parent.0,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
            let state = surface.prepare_overlay(&title);
            assert!(state.topmost);
            assert!(state.ready);
            assert_ne!(
                GetWindowLongPtrW(overlay.0, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
            SetWindowPos(
                parent.0,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
            let state = surface.prepare_overlay(&title);
            assert!(!state.topmost);
            assert_eq!(
                GetWindowLongPtrW(overlay.0, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST,
                0
            );
        }
    }
}
