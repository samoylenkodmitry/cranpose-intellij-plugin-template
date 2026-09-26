package dev.cranpose.intellij

import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.IOException
import java.net.Inet6Address
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.nio.file.Path
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.HexFormat
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/** Receives what a Cranpose process sends, on the session's reader thread. */
interface CranposeListener {
    fun onFrame(frame: AppEvent.Frame)

    fun onCursor(surface: Int, name: String) {}

    fun onMessage(channel: String, payload: String) {}

    /** The process opened, closed, or started moving or resizing one of its surfaces. */
    fun onCommand(command: SurfaceCommand) {}

    /** The process went away: closed by the host, exited, or crashed with [error]. */
    fun onExit(error: Throwable?) {}
}

/**
 * One running Cranpose process and its connection.
 *
 * The host listens on a loopback port, starts the process with the port and a
 * one-time token in its environment, and accepts only a connection that
 * answers with that token.
 */
class CranposeSession private constructor(
    private val process: Process,
    private val socket: Socket,
    private val reader: AppReader,
    val host: HostWriter,
) : AutoCloseable {
    @Volatile
    private var closed = false

    private fun startReading(listener: CranposeListener) {
        thread(name = "cranpose-session-reader", isDaemon = true) {
            val error = try {
                while (true) {
                    when (val event = reader.next() ?: break) {
                        is AppEvent.Frame -> listener.onFrame(event)
                        is AppEvent.Cursor -> listener.onCursor(event.surface, event.name)
                        is AppEvent.Message -> listener.onMessage(event.channel, event.payload)
                        is SurfaceCommand -> listener.onCommand(event)
                        is AppEvent.Hello -> Unit
                    }
                }
                null
            } catch (error: IOException) {
                error.takeUnless { closed }
            }
            listener.onExit(error)
        }
    }

    /** Asks the process to finish, then stops it if it does not. */
    override fun close() {
        if (closed) return
        closed = true
        runCatching { host.close() }
        runCatching { socket.close() }
        if (!process.waitFor(2, TimeUnit.SECONDS)) {
            process.destroy()
            if (!process.waitFor(2, TimeUnit.SECONDS)) process.destroyForcibly()
        }
    }

    companion object {
        const val ADDRESS_VARIABLE = "CRANPOSE_EMBED_ADDRESS"
        const val TOKEN_VARIABLE = "CRANPOSE_EMBED_TOKEN"
        private const val ACCEPT_POLL_MILLIS = 200

        /**
         * Starts [command], waits up to [connectTimeoutMillis] for it to connect
         * and prove itself, then delivers its events to [listener]. Every line
         * the process prints goes to [log]. Blocks; call it off the UI thread.
         */
        fun start(
            command: List<String>,
            workingDirectory: Path?,
            listener: CranposeListener,
            log: (String) -> Unit,
            connectTimeoutMillis: Int = 15_000,
            environment: Map<String, String> = emptyMap(),
        ): CranposeSession {
            val token = newToken()
            ServerSocket(0, 1, InetAddress.getLoopbackAddress()).use { server ->
                val process = ProcessBuilder(command)
                    .apply {
                        workingDirectory?.let { directory(it.toFile()) }
                        environment().putAll(environment)
                        environment()[ADDRESS_VARIABLE] = socketAddress(server)
                        environment()[TOKEN_VARIABLE] = token
                        redirectErrorStream(true)
                    }
                    .start()
                pumpOutput(process, log)
                val socket = try {
                    acceptWhileAlive(server, process, connectTimeoutMillis)
                } catch (error: IOException) {
                    process.destroyForcibly()
                    throw IOException("${command.first()} did not connect: ${error.message}", error)
                }
                try {
                    socket.tcpNoDelay = true
                    socket.soTimeout = connectTimeoutMillis
                    val reader = AppReader(BufferedInputStream(socket.getInputStream(), 1 shl 20))
                    val hello = reader.next() as? AppEvent.Hello
                        ?: throw IOException("the process did not introduce itself")
                    if (hello.version != CranposeProtocol.VERSION) {
                        throw IOException("the process speaks protocol ${hello.version}, the plugin ${CranposeProtocol.VERSION}")
                    }
                    if (!MessageDigest.isEqual(hello.token.toByteArray(), token.toByteArray())) {
                        throw IOException("the process answered with the wrong token")
                    }
                    socket.soTimeout = 0
                    val host = HostWriter(BufferedOutputStream(socket.getOutputStream(), 64 * 1024))
                    return CranposeSession(process, socket, reader, host).also { it.startReading(listener) }
                } catch (error: IOException) {
                    socket.close()
                    process.destroyForcibly()
                    throw error
                }
            }
        }

        private fun acceptWhileAlive(server: ServerSocket, process: Process, timeoutMillis: Int): Socket {
            val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis.toLong())
            server.soTimeout = ACCEPT_POLL_MILLIS
            while (true) {
                try {
                    return server.accept()
                } catch (_: SocketTimeoutException) {
                    if (!process.isAlive) throw IOException("the process exited with code ${process.exitValue()}")
                    if (System.nanoTime() > deadline) throw IOException("no connection within $timeoutMillis ms")
                }
            }
        }

        private fun socketAddress(server: ServerSocket): String {
            val address = server.inetAddress
            val host = if (address is Inet6Address) "[${address.hostAddress}]" else address.hostAddress
            return "$host:${server.localPort}"
        }

        private fun newToken(): String {
            val bytes = ByteArray(24)
            SecureRandom().nextBytes(bytes)
            return HexFormat.of().formatHex(bytes)
        }

        private fun pumpOutput(process: Process, log: (String) -> Unit) {
            thread(name = "cranpose-process-output", isDaemon = true) {
                process.inputStream.bufferedReader().useLines { lines -> lines.forEach(log) }
            }
        }
    }
}
