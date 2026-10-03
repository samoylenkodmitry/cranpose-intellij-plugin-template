import java.lang.reflect.InvocationTargetException;

/** Prints the texture interop capabilities of the runtime used to launch it. */
class SharedTextureCapabilities {
    public static void main(String[] args) throws Exception {
        System.out.println("os=" + System.getProperty("os.name"));
        System.out.println("runtime=" + System.getProperty("java.runtime.version"));
        try {
            Class<?> jbr = Class.forName("com.jetbrains.JBR");
            System.out.println("jbr=" + jbr.getMethod("isAvailable").invoke(null));
            boolean supported = (boolean) jbr.getMethod("isSharedTexturesSupported").invoke(null);
            System.out.println("sharedTextures=" + supported);
            if (supported) {
                Object service = jbr.getMethod("getSharedTextures").invoke(null);
                Class<?> textures = Class.forName("com.jetbrains.SharedTextures");
                System.out.println("textureType=" + textures.getMethod("getTextureType").invoke(service));
            }
        } catch (ClassNotFoundException | NoSuchMethodException e) {
            System.out.println("sharedTextures=false");
            System.out.println("reason=" + e);
        } catch (InvocationTargetException e) {
            System.out.println("sharedTextures=false");
            System.out.println("reason=" + e.getCause());
        }
        System.exit(0);
    }
}
