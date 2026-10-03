use super::*;
use windows::{
    Win32::{Foundation::HWND, Graphics::DirectComposition::*},
    core::{IUnknown, Interface},
};

pub struct Layer {
    device: IDCompositionDevice,
    target: IDCompositionTarget,
    visual: IDCompositionVisual,
}
// SAFETY: DirectComposition objects support calls from multiple threads. Target
// serializes all native updates and wgpu retains the visual for presentation.
unsafe impl Send for Layer {}
impl Layer {
    pub fn new(env: &mut JNIEnv<'_>, window: &JObject<'_>) -> Result<Self> {
        let hwnd = with_awt(env, window, |info| {
            Ok(info
                .surface_kind()
                .and_then(|kind| kind.window())
                .context("AWT HWND unavailable")?
                .0 as usize)
        })?;
        unsafe {
            // SAFETY: JAWT supplied this live HWND. The window is retained by Target.
            let device: IDCompositionDevice = DCompositionCreateDevice2(None::<&IUnknown>)?;
            let target = device.CreateTargetForHwnd(HWND(hwnd as *mut _), false)?;
            let visual = device.CreateVisual()?;
            target.SetRoot(&visual)?;
            device.Commit()?;
            Ok(Self {
                device,
                target,
                visual,
            })
        }
    }
    pub fn surface_target(&mut self) -> Result<wgpu::SurfaceTargetUnsafe> {
        Ok(wgpu::SurfaceTargetUnsafe::CompositionVisual(
            self.visual.as_raw(),
        ))
    }
    pub fn place(&mut self, b: Bounds) -> Result<()> {
        unsafe {
            self.visual.SetOffsetX2((b.x * b.scale) as f32)?;
            self.visual.SetOffsetY2((b.y * b.scale) as f32)?;
            self.device.Commit()?;
        }
        Ok(())
    }
    pub fn set_visible(&mut self, visible: bool) -> Result<()> {
        unsafe {
            self.target
                .SetRoot(if visible { Some(&self.visual) } else { None })?;
            self.device.Commit()?;
        }
        Ok(())
    }
    pub fn commit(&mut self) -> Result<()> {
        unsafe {
            self.device.Commit()?;
        }
        Ok(())
    }
}
impl Drop for Layer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.target.SetRoot(None::<&IDCompositionVisual>);
            let _ = self.device.Commit();
        }
    }
}
