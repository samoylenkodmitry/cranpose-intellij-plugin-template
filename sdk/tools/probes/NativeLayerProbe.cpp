#include <jni.h>
#include <jawt.h>
#include <jawt_md.h>
#include <memory>
#include <stdexcept>
#include <string>

static void fail(JNIEnv* env, const std::exception& error) {
    if (!env->ExceptionCheck()) env->ThrowNew(env->FindClass("java/lang/IllegalStateException"), error.what());
}

class DrawingSurface {
    JNIEnv* env;
    JAWT awt{};
    JAWT_DrawingSurface* surface = nullptr;
    JAWT_DrawingSurfaceInfo* info = nullptr;
    bool locked = false;
public:
    DrawingSurface(JNIEnv* env, jobject window) : env(env) {
        awt.version = JAWT_VERSION_9;
        if (!JAWT_GetAWT(env, &awt)) throw std::runtime_error("JAWT unavailable");
        surface = awt.GetDrawingSurface(env, window);
        if (!surface) throw std::runtime_error("JAWT drawing surface unavailable");
        if (!(surface->Lock(surface) & JAWT_LOCK_ERROR)) {
            locked = true;
            info = surface->GetDrawingSurfaceInfo(surface);
        }
        if (!info) {
            release();
            throw std::runtime_error("JAWT drawing surface could not be locked");
        }
    }
    ~DrawingSurface() { release(); }
    void* platform() const { return info->platformInfo; }
private:
    void release() {
        if (info) surface->FreeDrawingSurfaceInfo(info);
        if (locked) surface->Unlock(surface);
        if (surface) awt.FreeDrawingSurface(surface);
        info = nullptr;
        surface = nullptr;
        locked = false;
    }
};

#ifdef _WIN32
#include <d3d11.h>
#include <dxgi1_2.h>
#include <dcomp.h>
#include <wrl/client.h>
using Microsoft::WRL::ComPtr;

static void check(HRESULT result, const char* operation) {
    if (FAILED(result)) throw std::runtime_error(std::string(operation) + ": " + std::to_string(result));
}
struct Layer {
    ComPtr<ID3D11Device> device;
    ComPtr<ID3D11DeviceContext> context;
    ComPtr<IDCompositionDevice> composition;
    ComPtr<IDCompositionTarget> target;
    ComPtr<IDCompositionVisual> visual;
    ComPtr<IDXGISwapChain1> swapchain;
    int width = 0, height = 0;
    std::string name;
    explicit Layer(JNIEnv* env, jobject window) {
        DrawingSurface drawing(env, window);
        auto info = static_cast<JAWT_Win32DrawingSurfaceInfo*>(drawing.platform());
        D3D_FEATURE_LEVEL feature;
        HRESULT result = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0, D3D11_SDK_VERSION, &device, &feature, &context);
        name = "DirectComposition/D3D11 hardware";
        if (FAILED(result)) {
            check(D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_WARP, nullptr,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, nullptr, 0, D3D11_SDK_VERSION, &device, &feature, &context), "D3D11 WARP");
            name = "DirectComposition/D3D11 WARP (software validation only)";
        }
        ComPtr<IDXGIDevice> dxgi;
        check(device.As(&dxgi), "DXGI device");
        check(DCompositionCreateDevice(dxgi.Get(), IID_PPV_ARGS(&composition)), "DComposition device");
        check(composition->CreateTargetForHwnd(info->hwnd, TRUE, &target), "DComposition target");
        check(composition->CreateVisual(&visual), "DComposition visual");
        check(target->SetRoot(visual.Get()), "DComposition root");
    }
    ~Layer() {
        if (target) target->SetRoot(nullptr);
        if (composition) composition->Commit();
    }
    void present(int x, int y, int w, int h) {
        if (!swapchain) {
            ComPtr<IDXGIDevice> dxgi;
            ComPtr<IDXGIAdapter> adapter;
            ComPtr<IDXGIFactory2> factory;
            check(device.As(&dxgi), "DXGI device");
            check(dxgi->GetAdapter(&adapter), "DXGI adapter");
            check(adapter->GetParent(IID_PPV_ARGS(&factory)), "DXGI factory");
            DXGI_SWAP_CHAIN_DESC1 descriptor{};
            descriptor.Width = w; descriptor.Height = h;
            descriptor.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
            descriptor.SampleDesc.Count = 1;
            descriptor.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
            descriptor.BufferCount = 2;
            descriptor.Scaling = DXGI_SCALING_STRETCH;
            descriptor.SwapEffect = DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL;
            descriptor.AlphaMode = DXGI_ALPHA_MODE_PREMULTIPLIED;
            check(factory->CreateSwapChainForComposition(device.Get(), &descriptor, nullptr, &swapchain), "Composition swapchain");
            check(visual->SetContent(swapchain.Get()), "Visual content");
        } else if (width != w || height != h) {
            check(swapchain->ResizeBuffers(2, w, h, DXGI_FORMAT_UNKNOWN, 0), "Resize swapchain");
        }
        width = w; height = h;
        ComPtr<ID3D11Texture2D> buffer;
        ComPtr<ID3D11RenderTargetView> view;
        check(swapchain->GetBuffer(0, IID_PPV_ARGS(&buffer)), "Swapchain buffer");
        check(device->CreateRenderTargetView(buffer.Get(), nullptr, &view), "Render target");
        const float rgba[] = {0.125f, 0.25f, 0.5f, 0.5f};
        context->ClearRenderTargetView(view.Get(), rgba);
        check(swapchain->Present(1, 0), "Present");
        check(visual->SetOffsetX(static_cast<float>(x)), "Position X");
        check(visual->SetOffsetY(static_cast<float>(y)), "Position Y");
        check(composition->Commit(), "Commit");
    }
};
#else
#include <X11/extensions/Xfixes.h>
#include <X11/extensions/shape.h>
#include <X11/extensions/Xrender.h>
#include <X11/extensions/XTest.h>
#include <EGL/egl.h>
#include <GLES2/gl2.h>
#include <wayland-client.h>
#include <wayland-egl.h>
#include <cstring>
#include <cstdint>
#include <cstdlib>

struct Layer {
    Display* xdisplay = nullptr;
    Window xparent = 0;
    Window xwindow = 0;
    Colormap colormap = 0;
    wl_display* display = nullptr;
    wl_event_queue* queue = nullptr;
    wl_registry* registry = nullptr;
    wl_compositor* compositor = nullptr;
    wl_subcompositor* subcompositor = nullptr;
    wl_surface* parent = nullptr;
    wl_surface* child = nullptr;
    wl_subsurface* subsurface = nullptr;
    wl_egl_window* egl_window = nullptr;
    EGLDisplay egl = EGL_NO_DISPLAY;
    EGLContext context = EGL_NO_CONTEXT;
    EGLSurface surface = EGL_NO_SURFACE;
    EGLConfig config = nullptr;
    int width = 1, height = 1;
    std::string name;

    static void global(void* data, wl_registry* registry, uint32_t id, const char* interface, uint32_t version) {
        auto self = static_cast<Layer*>(data);
        if (!strcmp(interface, "wl_compositor")) self->compositor = static_cast<wl_compositor*>(wl_registry_bind(registry, id, &wl_compositor_interface, 1));
        if (!strcmp(interface, "wl_subcompositor")) self->subcompositor = static_cast<wl_subcompositor*>(wl_registry_bind(registry, id, &wl_subcompositor_interface, 1));
    }
    static void removed(void*, wl_registry*, uint32_t) {}
    Layer() = default;
    void initialize(JNIEnv* env, jobject window, bool wayland) {
        if (wayland) {
            jclass component = env->FindClass("java/awt/Component");
            jfieldID peerField = env->GetFieldID(component, "peer", "Ljava/awt/peer/ComponentPeer;");
            jobject peer = env->GetObjectField(window, peerField);
            if (!peer || env->ExceptionCheck()) throw std::runtime_error("AWT peer unavailable");
            jclass peerClass = env->GetObjectClass(peer);
            jmethodID getSurface = env->GetMethodID(peerClass, "getSurface", "()Lsun/awt/wl/WLMainSurface;");
            if (!getSurface) throw std::runtime_error("This JBR has no Wayland surface accessor");
            jobject mainSurface = env->CallObjectMethod(peer, getSurface);
            if (!mainSurface || env->ExceptionCheck()) throw std::runtime_error("JBR Wayland surface unavailable");
            jmethodID getPointer = env->GetMethodID(env->GetObjectClass(mainSurface), "getWlSurfacePtr", "()J");
            if (!getPointer) throw std::runtime_error("This JBR has no Wayland surface pointer accessor");
            parent = reinterpret_cast<wl_surface*>(env->CallLongMethod(mainSurface, getPointer));
            jclass displayClass = env->FindClass("sun/awt/wl/WLDisplay");
            if (!displayClass) throw std::runtime_error("This JBR has no Wayland display accessor");
            jmethodID getInstance = env->GetStaticMethodID(displayClass, "getInstance", "()Lsun/awt/wl/WLDisplay;");
            jmethodID getDisplay = env->GetMethodID(displayClass, "getDisplayPtr", "()J");
            if (!getInstance || !getDisplay) throw std::runtime_error("This JBR has no Wayland display pointer accessor");
            jobject instance = env->CallStaticObjectMethod(displayClass, getInstance);
            display = reinterpret_cast<wl_display*>(env->CallLongMethod(instance, getDisplay));
            if (!parent || !display || env->ExceptionCheck()) throw std::runtime_error("JBR returned no Wayland handles");
            queue = wl_display_create_queue(display);
            auto wrapper = static_cast<wl_display*>(wl_proxy_create_wrapper(display));
            wl_proxy_set_queue(reinterpret_cast<wl_proxy*>(wrapper), queue);
            registry = wl_display_get_registry(wrapper);
            wl_proxy_wrapper_destroy(wrapper);
            static const wl_registry_listener listener = {global, removed};
            wl_registry_add_listener(registry, &listener, this);
            if (wl_display_roundtrip_queue(display, queue) < 0 || !compositor || !subcompositor)
                throw std::runtime_error("Wayland subcompositor unavailable");
            child = wl_compositor_create_surface(compositor);
            subsurface = wl_subcompositor_get_subsurface(subcompositor, child, parent);
            wl_subsurface_set_desync(subsurface);
            wl_region* empty = wl_compositor_create_region(compositor);
            wl_surface_set_input_region(child, empty);
            wl_region_destroy(empty);
            egl_window = wl_egl_window_create(child, 1, 1);
            egl = eglGetDisplay(reinterpret_cast<EGLNativeDisplayType>(display));
            name = "Wayland/EGL";
        } else {
            DrawingSurface drawing(env, window);
            auto info = static_cast<JAWT_X11DrawingSurfaceInfo*>(drawing.platform());
            xdisplay = XOpenDisplay(DisplayString(info->display));
            if (!xdisplay) throw std::runtime_error("X11 display unavailable");
            xparent = info->drawable;
            XVisualInfo pattern{};
            pattern.screen = DefaultScreen(xdisplay); pattern.depth = 32; pattern.c_class = TrueColor;
            int count = 0;
            XVisualInfo* visuals = XGetVisualInfo(xdisplay, VisualScreenMask | VisualDepthMask | VisualClassMask, &pattern, &count);
            Visual* visual = nullptr;
            for (int i = 0; i < count; ++i) {
                XRenderPictFormat* format = XRenderFindVisualFormat(xdisplay, visuals[i].visual);
                if (format && format->type == PictTypeDirect && format->direct.alphaMask) { visual = visuals[i].visual; break; }
            }
            if (visuals) XFree(visuals);
            if (!visual) throw std::runtime_error("X11 ARGB visual unavailable");
            colormap = XCreateColormap(xdisplay, info->drawable, visual, AllocNone);
            XSetWindowAttributes attributes{};
            attributes.colormap = colormap;
            attributes.border_pixel = 0;
            attributes.background_pixel = 0;
            attributes.override_redirect = True;
            xwindow = XCreateWindow(xdisplay, DefaultRootWindow(xdisplay), 0, 0, 1, 1, 0, 32, InputOutput, visual,
                CWColormap | CWBorderPixel | CWBackPixel | CWOverrideRedirect, &attributes);
            XSetTransientForHint(xdisplay, xwindow, xparent);
            XserverRegion empty = XFixesCreateRegion(xdisplay, nullptr, 0);
            XFixesSetWindowShapeRegion(xdisplay, xwindow, ShapeInput, 0, 0, empty);
            XFixesDestroyRegion(xdisplay, empty);
            XMapWindow(xdisplay, xwindow);
            XSync(xdisplay, False);
            egl = eglGetDisplay(reinterpret_cast<EGLNativeDisplayType>(xdisplay));
            name = "X11 owned ARGB window/EGL";
        }
        EGLint major, minor;
        if (!eglInitialize(egl, &major, &minor)) throw std::runtime_error("EGL initialization failed");
        if (!eglBindAPI(EGL_OPENGL_ES_API)) throw std::runtime_error("EGL ES binding failed");
        EGLint attributes[] = {EGL_SURFACE_TYPE, EGL_WINDOW_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT,
            EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE};
        EGLint count;
        if (!eglChooseConfig(egl, attributes, &config, 1, &count) || !count) throw std::runtime_error("EGL alpha config unavailable");
        EGLint version[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
        context = eglCreateContext(egl, config, EGL_NO_CONTEXT, version);
        if (context == EGL_NO_CONTEXT) throw std::runtime_error("EGL context unavailable");
        auto native = egl_window ? reinterpret_cast<EGLNativeWindowType>(egl_window) : static_cast<EGLNativeWindowType>(xwindow);
        surface = eglCreateWindowSurface(egl, config, native, nullptr);
        if (surface == EGL_NO_SURFACE) throw std::runtime_error("EGL window surface unavailable");
        if (!eglMakeCurrent(egl, surface, surface, context)) throw std::runtime_error("EGL make current failed");
        name += "/";
        name += reinterpret_cast<const char*>(glGetString(GL_RENDERER));
        eglMakeCurrent(egl, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    }
    void present(int x, int y, int w, int h) {
        if (egl_window) {
            wl_subsurface_set_position(subsurface, x, y);
            wl_egl_window_resize(egl_window, w, h, 0, 0);
            wl_surface_commit(parent);
            wl_display_flush(display);
        } else {
            int rootX, rootY;
            Window child;
            XTranslateCoordinates(xdisplay, xparent, DefaultRootWindow(xdisplay), x, y, &rootX, &rootY, &child);
            XMoveResizeWindow(xdisplay, xwindow, rootX, rootY, w, h);
            XRaiseWindow(xdisplay, xwindow);
            XSync(xdisplay, False);
        }
        if (width != w || height != h) {
            eglDestroySurface(egl, surface);
            auto native = egl_window ? reinterpret_cast<EGLNativeWindowType>(egl_window) : static_cast<EGLNativeWindowType>(xwindow);
            surface = eglCreateWindowSurface(egl, config, native, nullptr);
            if (surface == EGL_NO_SURFACE) throw std::runtime_error("EGL resized surface unavailable");
            width = w; height = h;
        }
        if (!eglMakeCurrent(egl, surface, surface, context)) throw std::runtime_error("EGL make current failed");
        glViewport(0, 0, w, h);
        glClearColor(0.125f, 0.25f, 0.5f, 0.5f);
        glClear(GL_COLOR_BUFFER_BIT);
        if (!eglSwapBuffers(egl, surface)) throw std::runtime_error("EGL presentation failed");
        eglMakeCurrent(egl, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
    }
    ~Layer() {
        if (egl != EGL_NO_DISPLAY) {
            eglMakeCurrent(egl, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            if (surface != EGL_NO_SURFACE) eglDestroySurface(egl, surface);
            if (context != EGL_NO_CONTEXT) eglDestroyContext(egl, context);
            eglTerminate(egl);
        }
        if (egl_window) wl_egl_window_destroy(egl_window);
        if (subsurface) wl_subsurface_destroy(subsurface);
        if (child) wl_surface_destroy(child);
        if (subcompositor) wl_subcompositor_destroy(subcompositor);
        if (compositor) wl_compositor_destroy(compositor);
        if (registry) wl_registry_destroy(registry);
        if (queue) wl_event_queue_destroy(queue);
        if (display) wl_display_flush(display);
        if (xwindow) XDestroyWindow(xdisplay, xwindow);
        if (colormap) XFreeColormap(xdisplay, colormap);
        if (xdisplay) XCloseDisplay(xdisplay);
    }
};
#endif

extern "C" JNIEXPORT jlong JNICALL Java_NativeLayerProbe_create(JNIEnv* env, jclass, jobject window, jboolean wayland) {
    try {
#ifdef _WIN32
        auto layer = std::make_unique<Layer>(env, window);
#else
        auto layer = std::make_unique<Layer>();
        layer->initialize(env, window, wayland);
#endif
        return reinterpret_cast<jlong>(layer.release());
    } catch (const std::exception& error) { fail(env, error); return 0; }
}
extern "C" JNIEXPORT jstring JNICALL Java_NativeLayerProbe_backend(JNIEnv* env, jclass, jlong handle) {
    return env->NewStringUTF(reinterpret_cast<Layer*>(handle)->name.c_str());
}
extern "C" JNIEXPORT void JNICALL Java_NativeLayerProbe_present(JNIEnv* env, jclass, jlong handle, jint x, jint y, jint w, jint h) {
    try {
        if (!handle || w < 1 || h < 1) throw std::runtime_error("Invalid layer or bounds");
        reinterpret_cast<Layer*>(handle)->present(x, y, w, h);
    } catch (const std::exception& error) { fail(env, error); }
}
extern "C" JNIEXPORT void JNICALL Java_NativeLayerProbe_destroy(JNIEnv*, jclass, jlong handle) {
    delete reinterpret_cast<Layer*>(handle);
}
extern "C" JNIEXPORT jboolean JNICALL Java_NativeLayerProbe_clickWayland(JNIEnv* env, jclass, jint x, jint y) {
#ifdef _WIN32
    return false;
#else
    const char* parentDisplay = std::getenv("CRANPOSE_PROBE_PARENT_DISPLAY");
    if (!parentDisplay) return false;
    Display* display = XOpenDisplay(parentDisplay);
    if (!display) return false;
    Window root, parent, *children = nullptr;
    unsigned count = 0;
    bool result = false;
    Window compositor = None;
    if (XQueryTree(display, DefaultRootWindow(display), &root, &parent, &children, &count)) {
        for (unsigned i = 0; i < count; ++i) {
            XWindowAttributes attributes{};
            if (XGetWindowAttributes(display, children[i], &attributes) && attributes.map_state == IsViewable) {
                if (compositor != None) { compositor = None; break; }
                compositor = children[i];
            }
        }
    }
    if (compositor != None) {
        int rootX, rootY;
        Window child;
        XTranslateCoordinates(display, compositor, root, x, y, &rootX, &rootY, &child);
        XTestFakeMotionEvent(display, -1, rootX, rootY, CurrentTime);
        XTestFakeButtonEvent(display, 1, True, CurrentTime);
        XTestFakeButtonEvent(display, 1, False, CurrentTime);
        XSync(display, False);
        result = true;
    }
    if (children) XFree(children);
    XCloseDisplay(display);
    return result;
#endif
}
