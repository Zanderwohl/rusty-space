//! The desktop app icons, rendered by `tools/build-app-icons.sh` into `icons/<name>/` as PNGs,
//! an `.icns` and an `.ico` for packaging.
//!
//! [`AppIconPlugin`] sets one on the running app: the window icon on Windows and X11, and the
//! Dock icon on macOS. Wayland takes icons only from a `.desktop` file, so it ignores both.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::winit::WINIT_WINDOWS;

/// The PNGs a running app needs. A binary embeds only the icon it names, since the linker drops
/// the bytes of the other.
#[derive(Clone, Copy)]
pub struct AppIcon {
    /// 256 px: big enough for the Windows taskbar at 200% scale and an X11 window switcher.
    window_png: &'static [u8],
    /// 1024 px, for the Dock on a Retina display.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    dock_png: &'static [u8],
}

macro_rules! app_icon {
    ($dir:literal) => {
        AppIcon {
            window_png: include_bytes!(concat!("../icons/", $dir, "/icon-256.png")),
            dock_png: include_bytes!(concat!("../icons/", $dir, "/icon-1024.png")),
        }
    };
}

/// The site's micro-galaxy, 30 degrees above its plane. Galaxy Forge's.
pub const GALAXY: AppIcon = app_icon!("galaxy-30");
/// The site's green favicon. The desktop client's.
pub const LIGHTCONE: AppIcon = app_icon!("lightcone-green");

pub struct AppIconPlugin(pub AppIcon);

impl Plugin for AppIconPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Chosen(self.0));
        #[cfg(target_os = "macos")]
        app.add_systems(Startup, set_dock_icon);
        app.add_systems(Update, set_window_icon.run_if(not(resource_exists::<WindowIconSet>)));
    }
}

#[derive(Resource)]
struct Chosen(AppIcon);

#[derive(Resource)]
struct WindowIconSet;

/// The winit window appears a frame or more after startup, so this retries until it exists.
fn set_window_icon(
    mut commands: Commands,
    icon: Res<Chosen>,
    primary: Query<Entity, With<PrimaryWindow>>,
    _main_thread: bevy::ecs::system::NonSendMarker,
) {
    let Ok(entity) = primary.single() else { return };
    let set = WINIT_WINDOWS.with_borrow(|windows| {
        let Some(window) = windows.get_window(entity) else { return false };
        window.set_window_icon(window_icon(icon.0.window_png));
        true
    });
    if set {
        commands.insert_resource(WindowIconSet);
    }
}

fn window_icon(png: &[u8]) -> Option<winit::window::Icon> {
    let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .inspect_err(|e| warn!("window icon: {e}"))
        .ok()?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    winit::window::Icon::from_rgba(rgba.into_raw(), width, height)
        .inspect_err(|e| warn!("window icon: {e}"))
        .ok()
}

/// Replaces the bundle's icon in the Dock while the app runs. Finder, and the Dock before
/// launch, still read the `.icns` named by the bundle's `Info.plist`.
#[cfg(target_os = "macos")]
fn set_dock_icon(icon: Res<Chosen>, _main_thread: bevy::ecs::system::NonSendMarker) {
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(icon.0.dock_png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        warn!("dock icon: AppKit could not decode the PNG");
        return;
    };
    // SAFETY: on the main thread, with a non-nil image.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}
