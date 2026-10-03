use super::*;
#[path = "linux/wayland.rs"]
mod wayland;
#[path = "linux/x11.rs"]
mod x11;
pub enum Layer {
    X11(Box<x11::Layer>),
    Wayland(Box<wayland::Layer>),
}
impl Layer {
    pub fn new(env: &mut JNIEnv<'_>, window: &JObject<'_>) -> Result<Self> {
        let toolkit = env
            .call_static_method(
                "java/awt/Toolkit",
                "getDefaultToolkit",
                "()Ljava/awt/Toolkit;",
                &[],
            )?
            .l()?;
        let class = env
            .call_method(toolkit, "getClass", "()Ljava/lang/Class;", &[])?
            .l()?;
        let name = env
            .call_method(class, "getName", "()Ljava/lang/String;", &[])?
            .l()?;
        let name = String::from(env.get_string(&JString::from(name))?);
        if name == "sun.awt.wl.WLToolkit" {
            Ok(Self::Wayland(Box::new(wayland::Layer::new(env, window)?)))
        } else if name == "sun.awt.X11.XToolkit" {
            Ok(Self::X11(Box::new(x11::Layer::new(env, window)?)))
        } else {
            anyhow::bail!("Unsupported AWT toolkit: {name}")
        }
    }
    pub fn surface_target(&mut self) -> Result<wgpu::SurfaceTargetUnsafe> {
        match self {
            Self::X11(l) => l.surface_target(),
            Self::Wayland(l) => l.surface_target(),
        }
    }
    pub fn place(&mut self, b: Bounds) -> Result<()> {
        match self {
            Self::X11(l) => l.place(b),
            Self::Wayland(l) => l.place(b),
        }
    }
    pub fn set_visible(&mut self, visible: bool) -> Result<()> {
        match self {
            Self::X11(l) => l.set_visible(visible),
            Self::Wayland(l) => l.set_visible(visible),
        }
    }
    pub fn commit(&mut self) -> Result<()> {
        match self {
            Self::X11(l) => l.commit(),
            Self::Wayland(l) => l.commit(),
        }
    }
}
