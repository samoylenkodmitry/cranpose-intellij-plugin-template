use super::*;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, XlibDisplayHandle, XlibWindowHandle};
use std::{ffi::CString, ptr};
use x11_dl::{xfixes, xlib, xrender};

pub struct Layer {
    api: xlib::Xlib,
    display: *mut xlib::Display,
    parent: u64,
    window: u64,
    colormap: u64,
    screen: i32,
    visual: u64,
}
// SAFETY: owns its own X connection. Target serializes calls; XInitThreads is
// already required by AWT, and wgpu serializes its own use of this display.
unsafe impl Send for Layer {}
impl Layer {
    pub fn new(env: &mut JNIEnv<'_>, window: &JObject<'_>) -> Result<Self> {
        let api = xlib::Xlib::open()?;
        let render = xrender::Xrender::open()?;
        let fixes = xfixes::Xlib::open()?;
        let (name, parent) = with_awt(env, window, |info| unsafe {
            Ok((
                CString::from(std::ffi::CStr::from_ptr((api.XDisplayString)(
                    info.display(),
                ))),
                info.window()?,
            ))
        })?;
        unsafe {
            // SAFETY: names originate from the locked JAWT surface. All allocated
            // X resources belong to this connection and are freed by Layer.
            let display = (api.XOpenDisplay)(name.as_ptr());
            ensure!(!display.is_null(), "Cannot open AWT's X display");
            let mut layer = Self {
                api,
                display,
                parent,
                window: 0,
                colormap: 0,
                screen: 0,
                visual: 0,
            };
            layer.screen = (layer.api.XDefaultScreen)(display);
            let root = (layer.api.XRootWindow)(display, layer.screen);
            let selection = CString::new(format!("_NET_WM_CM_S{}", layer.screen))?;
            let atom = (layer.api.XInternAtom)(display, selection.as_ptr(), 0);
            ensure!(
                (layer.api.XGetSelectionOwner)(display, atom) != 0,
                "X11 transparency requires a compositor"
            );
            let mut template: xlib::XVisualInfo = std::mem::zeroed();
            template.screen = layer.screen;
            template.depth = 32;
            template.class = xlib::TrueColor;
            let mut count = 0;
            let list = (layer.api.XGetVisualInfo)(
                display,
                xlib::VisualScreenMask | xlib::VisualDepthMask | xlib::VisualClassMask,
                &mut template,
                &mut count,
            );
            ensure!(!list.is_null(), "X11 ARGB visual unavailable");
            let mut visual = ptr::null_mut();
            for info in std::slice::from_raw_parts(list, count as usize) {
                let format = (render.XRenderFindVisualFormat)(display, info.visual);
                if !format.is_null() && (*format).direct.alphaMask != 0 {
                    visual = info.visual;
                    layer.visual = info.visualid;
                    break;
                }
            }
            (layer.api.XFree)(list.cast());
            ensure!(!visual.is_null(), "X11 ARGB visual unavailable");
            layer.colormap = (layer.api.XCreateColormap)(display, root, visual, xlib::AllocNone);
            let mut attributes: xlib::XSetWindowAttributes = std::mem::zeroed();
            attributes.colormap = layer.colormap;
            attributes.override_redirect = 1;
            layer.window = (layer.api.XCreateWindow)(
                display,
                root,
                0,
                0,
                1,
                1,
                0,
                32,
                xlib::InputOutput as u32,
                visual,
                xlib::CWColormap
                    | xlib::CWBorderPixel
                    | xlib::CWBackPixel
                    | xlib::CWOverrideRedirect,
                &mut attributes,
            );
            ensure!(layer.window != 0, "X11 layer creation failed");
            (layer.api.XSetTransientForHint)(display, layer.window, parent);
            let empty = (fixes.XFixesCreateRegion)(display, ptr::null_mut(), 0);
            (fixes.XFixesSetWindowShapeRegion)(display, layer.window, 2, 0, 0, empty);
            (fixes.XFixesDestroyRegion)(display, empty);
            (layer.api.XSync)(display, 0);
            Ok(layer)
        }
    }
    pub fn surface_target(&self) -> Result<wgpu::SurfaceTargetUnsafe> {
        let mut window = XlibWindowHandle::new(self.window);
        window.visual_id = self.visual;
        Ok(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Xlib(XlibDisplayHandle::new(
                std::ptr::NonNull::new(self.display.cast()),
                self.screen,
            ))),
            raw_window_handle: RawWindowHandle::Xlib(window),
        })
    }
    pub fn place(&mut self, b: Bounds) -> Result<()> {
        let [w, h] = b.pixels();
        unsafe {
            let root = (self.api.XRootWindow)(self.display, self.screen);
            let (mut x, mut y, mut child) = (0, 0, 0);
            ensure!(
                (self.api.XTranslateCoordinates)(
                    self.display,
                    self.parent,
                    root,
                    (b.x * b.scale).round() as i32,
                    (b.y * b.scale).round() as i32,
                    &mut x,
                    &mut y,
                    &mut child
                ) != 0,
                "X11 owner is unavailable"
            );
            (self.api.XMoveResizeWindow)(self.display, self.window, x, y, w, h);
            (self.api.XFlush)(self.display);
        }
        Ok(())
    }
    pub fn set_visible(&mut self, visible: bool) -> Result<()> {
        unsafe {
            if visible {
                (self.api.XMapRaised)(self.display, self.window);
            } else {
                (self.api.XUnmapWindow)(self.display, self.window);
            }
        }
        self.commit()
    }
    pub fn commit(&mut self) -> Result<()> {
        unsafe {
            (self.api.XFlush)(self.display);
        }
        Ok(())
    }
}
impl Drop for Layer {
    fn drop(&mut self) {
        unsafe {
            if self.window != 0 {
                (self.api.XDestroyWindow)(self.display, self.window);
            }
            if self.colormap != 0 {
                (self.api.XFreeColormap)(self.display, self.colormap);
            }
            (self.api.XCloseDisplay)(self.display);
        }
    }
}
