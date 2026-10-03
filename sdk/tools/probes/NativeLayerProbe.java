import java.awt.*;
import java.awt.event.InputEvent;
import java.util.concurrent.atomic.AtomicInteger;
import javax.swing.*;

/** Exercises a native transparent GPU layer over Swing without transferring frame pixels. */
class NativeLayerProbe {
    private static native long create(Window window, boolean wayland);
    private static native String backend(long layer);
    private static native void present(long layer, int x, int y, int width, int height);
    private static native void destroy(long layer);

    private static JFrame frame;
    private static JPanel content;
    private static long layer;
    private static final AtomicInteger clicks = new AtomicInteger();

    public static void main(String[] args) {
        try {
            System.load(args[0]);
            EventQueue.invokeAndWait(() -> {
                frame = new JFrame("Cranpose GPU presentation probe");
                frame.setUndecorated(true);
                frame.setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
                content = new JPanel(null);
                content.setBackground(Color.WHITE);
                JButton button = new JButton("Input under the GPU layer");
                button.setBounds(32, 120, 256, 48);
                button.addActionListener(event -> clicks.incrementAndGet());
                content.add(button);
                frame.setContentPane(content);
                frame.setBounds(100, 100, 400, 240);
                frame.setVisible(true);
                layer = create(frame, Toolkit.getDefaultToolkit().getClass().getName().contains("WLToolkit"));
                if (layer == 0) throw new AssertionError("Native layer creation returned no handle");
                System.out.println("backend=" + backend(layer));
                System.out.println("toolkit=" + Toolkit.getDefaultToolkit().getClass().getName());
                System.out.println("runtime=" + System.getProperty("java.runtime.version"));
            });
            Robot robot = new Robot();
            robot.setAutoDelay(40);
            double scale = frame.getGraphicsConfiguration().getDefaultTransform().getScaleX();
            System.out.println("displayScale=" + scale + ", location=" + frame.getLocationOnScreen());
            robot.waitForIdle();
            awaitPixel(robot, 64, 40, Color.WHITE);
            for (int y : new int[] { 32, 72, 32 }) {
                EventQueue.invokeAndWait(() -> present(layer, (int)(32 * scale), (int)(y * scale),
                    (int)(256 * scale), (int)(32 * scale)));
                awaitPixel(robot, 64, y + 8, new Color(159, 191, 255));
                awaitPixel(robot, 24, y + 8, Color.WHITE);
                awaitPixel(robot, 64, y - 8, Color.WHITE);
            }
            EventQueue.invokeAndWait(() -> present(layer, (int)(32 * scale), (int)(120 * scale),
                (int)(256 * scale), (int)(48 * scale)));
            Point origin = frame.getLocationOnScreen();
            robot.mouseMove(origin.x + 160, origin.y + 144);
            robot.mousePress(InputEvent.BUTTON1_DOWN_MASK);
            robot.mouseRelease(InputEvent.BUTTON1_DOWN_MASK);
            robot.waitForIdle();
            if (clicks.get() != 1) throw new AssertionError("GPU layer intercepted input: " + clicks.get());
            System.out.println("PASS: native GPU alpha, translated placement, bounds and input passthrough");
            System.out.println("pixelTransport=none (GPU clear and native presentation; screen readback is test-only)");
            System.out.println("displayScale=" + scale);
        } catch (Throwable failure) {
            failure.printStackTrace();
            cleanup();
            System.exit(1);
        }
        cleanup();
        System.exit(0);
    }

    private static void awaitPixel(Robot robot, int x, int y, Color expected) throws Exception {
        long deadline = System.nanoTime() + 3_000_000_000L;
        Color actual;
        do {
            Point origin = frame.getLocationOnScreen();
            actual = robot.getPixelColor(origin.x + x, origin.y + y);
            if (Math.abs(actual.getRed() - expected.getRed()) <= 5
                && Math.abs(actual.getGreen() - expected.getGreen()) <= 5
                && Math.abs(actual.getBlue() - expected.getBlue()) <= 5) return;
            Thread.sleep(40);
        } while (System.nanoTime() < deadline);
        throw new AssertionError("Pixel " + x + "," + y + ": " + actual + ", expected " + expected);
    }

    private static void cleanup() {
        try {
            EventQueue.invokeAndWait(() -> {
                if (layer != 0) {
                    destroy(layer);
                    layer = 0;
                }
                if (frame != null) frame.dispose();
            });
        } catch (Throwable failure) {
            failure.printStackTrace();
        }
    }
}
