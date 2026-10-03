import com.jetbrains.JBR;
import java.awt.AlphaComposite;
import java.awt.Color;
import java.awt.Graphics2D;
import java.awt.GraphicsEnvironment;
import java.awt.Image;
import java.awt.Transparency;
import java.awt.image.BufferedImage;
import java.awt.image.VolatileImage;

/** Exercises GPU texture import, clipping, alpha and presentation without a pixel upload. */
class SharedTextureProbe {
    private static native long create();
    private static native void release(long texture);

    public static void main(String[] args) throws Exception {
        try {
            run(args);
        } catch (Throwable failure) {
            failure.printStackTrace();
            System.exit(1);
        }
        System.exit(0);
    }

    private static void run(String[] args) throws Exception {
        System.load(args[0]);
        if (!JBR.isSharedTexturesSupported()) throw new AssertionError("Shared textures unavailable");
        var gc = GraphicsEnvironment.getLocalGraphicsEnvironment().getDefaultScreenDevice().getDefaultConfiguration();
        long texture = create();
        if (texture == 0) throw new AssertionError("Metal rendering failed");
        Image image = null;
        VolatileImage target = gc.createCompatibleVolatileImage(128, 64, Transparency.TRANSLUCENT);
        try {
            image = JBR.getSharedTextures().wrapTexture(gc, texture);
            double scale = gc.getDefaultTransform().getScaleX();
            if (image.getWidth(null) != (int) (64 / scale) || image.getHeight(null) != (int) (32 / scale)) throw new AssertionError("Texture dimensions: " + image.getWidth(null) + "x" + image.getHeight(null));
            for (int y : new int[] { 12, 4 }) {
                target.validate(gc);
                Graphics2D g = target.createGraphics();
                try {
                    g.setComposite(AlphaComposite.Src);
                    g.setColor(Color.WHITE);
                    g.fillRect(0, 0, 128, 64);
                    g.setComposite(AlphaComposite.SrcOver);
                    g.setClip(16, 0, 32, 64);
                    if (!g.drawImage(image, 8, y, null)) throw new AssertionError("Image was not ready");
                } finally {
                    g.dispose();
                }
                // Readback is confined to validation. Production presentation uses drawImage.
                BufferedImage pixels = target.getSnapshot();
                int actual = pixels.getRGB(20, y + 8);
                if (Math.abs(((actual >> 16) & 255) - 159) > 2
                        || Math.abs(((actual >> 8) & 255) - 191) > 2
                        || (actual & 255) < 253) throw new AssertionError("Alpha/color: " + Integer.toHexString(actual));
                if (pixels.getRGB(12, y + 8) != Color.WHITE.getRGB()) throw new AssertionError("Clip escaped");
                if (pixels.getRGB(20, y - 1) != Color.WHITE.getRGB()) throw new AssertionError("Translated bounds escaped");
            }
            System.out.println("PASS: GPU texture import, dimensions, premultiplied alpha, clipping and translated placement");
            System.out.println("acceleratedTarget=" + target.getCapabilities().isAccelerated());
            System.out.println("displayScale=" + scale);
        } finally {
            target.flush();
            if (image != null) image.flush();
            release(texture);
        }
        System.exit(0);
    }
}
