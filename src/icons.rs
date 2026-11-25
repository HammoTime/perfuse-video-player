//! SVG icons bundled from `assets/` and rasterised by egui_extras at display size.

use eframe::egui::{self, Color32, Vec2};

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        pub enum Icon {
            $($name,)*
        }

        const SOURCES: &[(&str, &str)] = &[
            $((concat!("bytes://perfuse/", $file), include_str!(concat!("../assets/", $file))),)*
        ];

        impl Icon {
            fn uri(self) -> &'static str {
                match self {
                    $(Self::$name => concat!("bytes://perfuse/", $file),)*
                }
            }
        }
    };
}

icons! {
    Alert => "icons/alert.svg",
    AudioTrack => "icons/audio-track.svg",
    Back => "icons/back.svg",
    Back10 => "icons/back-10.svg",
    Cancel => "icons/cancel.svg",
    ChapterNext => "icons/chapter-next.svg",
    ChapterPrevious => "icons/chapter-previous.svg",
    Clear => "icons/clear.svg",
    Close => "icons/close.svg",
    Connect => "icons/connect.svg",
    Dismiss => "icons/dismiss.svg",
    Forget => "icons/forget.svg",
    Forward10 => "icons/forward-10.svg",
    Fullscreen => "icons/fullscreen.svg",
    FullscreenExit => "icons/fullscreen-exit.svg",
    Home => "icons/home.svg",
    Maximize => "icons/maximize.svg",
    Minimize => "icons/minimize.svg",
    Movies => "icons/movies.svg",
    PageNext => "icons/page-next.svg",
    PagePrevious => "icons/page-previous.svg",
    Pause => "icons/pause.svg",
    Pin => "icons/pin.svg",
    Play => "icons/play.svg",
    PlayFromBeginning => "icons/play-from-beginning.svg",
    Refresh => "icons/refresh.svg",
    Restore => "icons/restore.svg",
    Resume => "icons/resume.svg",
    Search => "icons/search.svg",
    Server => "icons/server.svg",
    Settings => "icons/settings.svg",
    SignInBrowser => "icons/sign-in-browser.svg",
    SignOut => "icons/sign-out.svg",
    Sort => "icons/sort.svg",
    Speed => "icons/speed.svg",
    StatusConnected => "icons/status-connected.svg",
    Subtitles => "icons/subtitles.svg",
    TvShows => "icons/tv-shows.svg",
    VolumeHigh => "icons/volume-high.svg",
    VolumeLow => "icons/volume-low.svg",
    VolumeMuted => "icons/volume-muted.svg",
    Watched => "icons/watched.svg",
    Mark => "brand/perfuse-mark.svg",
}

/// Registers the SVG loader and every bundled icon with this context.
pub fn install(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
    for (uri, svg) in SOURCES {
        // Icons draw in `currentColor`, which resvg resolves to black. Render
        // them white so the egui tint becomes the visible colour.
        ctx.include_bytes(*uri, svg.replace("currentColor", "#FFFFFF").into_bytes());
    }
}

/// The window icon, rasterised from the brand SVG.
pub fn app_icon() -> Option<egui::IconData> {
    use resvg::{tiny_skia, usvg};
    let svg = include_str!("../assets/brand/perfuse-app-icon.svg");
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let size = 64;
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia stores premultiplied colour; window icons take straight alpha.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Some(egui::IconData {
        rgba,
        width: size,
        height: size,
    })
}

/// eframe only sets the small window icon, which the taskbar would scale up.
/// Give the window the large icon that build.rs embeds as resource 1.
#[cfg(windows)]
pub fn use_large_icon(window: &impl raw_window_handle::HasWindowHandle) {
    use windows_sys::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    };
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let raw_window_handle::RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as _;
    // SAFETY: the handle is eframe's live root window on this UI thread, and
    // resource 1 is this executable's icon. The icon stays loaded for the
    // lifetime of the process, as the window keeps using it.
    unsafe {
        let size = GetSystemMetricsForDpi(SM_CXICON, GetDpiForWindow(hwnd));
        let icon = LoadImageW(
            GetModuleHandleW(std::ptr::null()),
            1 as _,
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        );
        if !icon.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        }
    }
}

impl Icon {
    /// A square icon image of `size` points in `color`. Coloured artwork such
    /// as the brand mark keeps its own colours when tinted white.
    pub fn image(self, size: f32, color: Color32) -> egui::Image<'static> {
        egui::Image::from_uri(self.uri())
            .fit_to_exact_size(Vec2::splat(size))
            .tint(color)
    }

    pub fn paint(self, ui: &egui::Ui, center: egui::Pos2, size: f32, color: Color32) {
        self.image(size, color)
            .paint_at(ui, egui::Rect::from_center_size(center, Vec2::splat(size)));
    }
}

#[cfg(test)]
mod tests {
    use super::{SOURCES, install};
    use eframe::egui::{self, load::ImagePoll};

    #[test]
    fn bundled_icons_render_visible_pixels() {
        let ctx = egui::Context::default();
        install(&ctx);
        for (uri, _) in SOURCES {
            let Ok(ImagePoll::Ready { image }) = ctx.try_load_image(uri, Default::default()) else {
                panic!("{uri} did not render");
            };
            // A white icon proves `currentColor` was replaced before tinting.
            let brand = uri.contains("/brand/");
            assert!(
                image
                    .pixels
                    .iter()
                    .any(|p| p.a() > 0 && (brand || p.r() == p.a())),
                "{uri} rendered no tintable pixels"
            );
        }
    }
}
