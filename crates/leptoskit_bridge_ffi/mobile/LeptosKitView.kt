package dev.leptoskit.mobile

import android.content.Context
import android.content.res.Configuration
import android.os.Handler
import android.os.Looper
import android.widget.FrameLayout
import android.widget.TextView
import dev.leptoskit.bridge.NativeBridge
import dev.leptoskit.bridge.NativeDispatcher
import kotlinx.coroutines.Job
import kotlinx.coroutines.joinAll

internal class NativeApplicationInitializationFailure(
    cause: Exception,
    val join: suspend () -> Unit,
) : IllegalStateException(cause.message, cause)

/** UIKit's counterpart: real Android controls driven by the Rust renderer. */
class LeptosKitView(
    context: Context,
    wasm: ByteArray,
    private val onReady: () -> Unit = {},
    private val onFailure: ((Exception) -> Unit)? = null,
    register: (Long, NativeDispatcher) -> Unit,
) : FrameLayout(context),
    AutoCloseable {
    private companion object {
        const val MAX_PUMP_BATCH = 64
        const val REQUEST_DEFERRED = 2
    }

    private var app = 0L
    private var riveAnimator: NativeRiveAnimator? = null
    private val dispatcher = NativeDispatcher()
    private val handler = Handler(Looper.getMainLooper())
    private val services =
        handler.let { main ->
            NativeServiceOwner(
                post = { task -> check(main.post { task() }) { "Main looper stopped" } },
                poll = NativeApp::pollClose,
                release = NativeApp::releaseClose,
                active = { if (app != 0L) runUi { pump() } },
            )
        }
    private val jobs =
        NativePluginJobs(
            { app },
            dispatcher,
            handler,
            ::runUi,
            ::checkStatus,
            ::schedulePump,
        )
    private var closingJobs: List<Job> = emptyList()
    private var closeFailure: Exception? = null
    private var closeStarted = false
    private var pumpScheduled = false
    private val renderer = NativeWidgetRenderer(this, ::event) { onCanvasChanged?.invoke(this) }

    /** The root's opaque background, for the host to paint behind system bars. */
    val canvas: Int? get() = renderer.canvas

    /** Called on the UI thread when [canvas] changes. */
    var onCanvasChanged: ((LeptosKitView) -> Unit)? = null
    private var started = false
    private var ready = false
    private var readyScheduled = false
    private val density get() = resources.displayMetrics.density

    init {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Application creation requires the main thread" }
        NativeBridge.load()
        try {
            app =
                NativeApp.create(
                    wasm,
                    context.filesDir.absolutePath.toByteArray(Charsets.UTF_8),
                    context.cacheDir.absolutePath.toByteArray(Charsets.UTF_8),
                )
            check(app != 0L) { "Cannot create application" }
            checkStatus(NativeApp.setTextMeasurer(app, NativeTextMeasurer(resources.displayMetrics)))
            checkStatus(NativeApp.activateService(app, services))
            register(NativeApp.plugins(app), dispatcher)
            riveAnimator =
                NativeRiveAnimator(
                    this,
                    hasScenes = { app != 0L && NativeApp.hasRive(app) },
                    tick = { elapsed, scale, flags ->
                        if (app == 0L) {
                            false
                        } else {
                            val status = NativeApp.riveTick(app, elapsed, scale, flags, renderer)
                            checkStatus(status)
                            status == 1
                        }
                    },
                    fail = { error -> runUi { throw error } },
                )
            renderer.onRiveViewportChange = { riveAnimator?.requestFrame() }
        } catch (error: Exception) {
            close()
            throw NativeApplicationInitializationFailure(error) { closeAndJoin() }
        }
    }

    /** Stop admission before transferring App ownership; plugin joins remain awaitable. */
    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Application close requires the main thread" }

        if (closeStarted) return
        closeStarted = true
        riveAnimator?.close()
        services.stop()
        val owned = app
        app = 0L
        closingJobs = jobs.close()
        try {
            dispatcher.close()
        } catch (error: Exception) {
            closeFailure = error
        } finally {
            if (owned != 0L) services.close(NativeApp.beginClose(owned))
            services.finishAdmission()
            renderer.clear()
        }
    }

    /** Cancelling this await never cancels the retained native close owner. */
    suspend fun closeAndJoin() {
        close()
        closingJobs.joinAll()
        services.awaitClosed()
        closeFailure?.let { throw it }
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        riveAnimator?.requestFrame()
    }

    override fun onDetachedFromWindow() {
        riveAnimator?.detach()
        super.onDetachedFromWindow()
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        riveAnimator?.requestFrame()
    }

    override fun onWindowVisibilityChanged(visibility: Int) {
        super.onWindowVisibilityChanged(visibility)
        riveAnimator?.requestFrame()
    }

    override fun onVisibilityChanged(
        changedView: android.view.View,
        visibility: Int,
    ) {
        super.onVisibilityChanged(changedView, visibility)
        riveAnimator?.requestFrame()
    }

    override fun onSizeChanged(
        width: Int,
        height: Int,
        oldWidth: Int,
        oldHeight: Int,
    ) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)

        if (app == 0L || width <= 0 || height <= 0) return
        // Adding children during the current layout pass can discard their
        // requestLayout. Render on the next UI turn so Android measures them.
        post {
            if (app == 0L) return@post
            runUi { resizeGuest(width, height) }
        }
    }

    override fun onConfigurationChanged(configuration: Configuration) {
        super.onConfigurationChanged(configuration)
        // Density and display preferences can change without a pixel-size change.
        post {
            if (app != 0L && width > 0 && height > 0) runUi { resizeGuest(width, height) }
        }
    }

    private fun resizeGuest(
        width: Int,
        height: Int,
    ) {
        if (!started) {
            started = true
            checkStatus(NativeApp.start(app, width / density, height / density))
        } else {
            val dark =
                resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
                    Configuration.UI_MODE_NIGHT_YES
            checkStatus(NativeApp.resize(app, width / density, height / density, dark))
        }

        pump()
    }

    private fun checkStatus(status: Int) {
        check(status >= 0) { NativeApp.error(app).toString(Charsets.UTF_8) }
    }

    private fun runUi(block: () -> Unit) {
        try {
            block()
        } catch (error: Exception) {
            close()
            removeAllViews()

            if (onFailure != null) {
                onFailure.invoke(error)

                return
            }

            addView(
                TextView(context).apply {
                    text = error.message ?: error.javaClass.simpleName
                    contentDescription = "leptoskit-error"
                },
            )
        }
    }

    /** Poll service work first, then bounded cancellation/request batches before rendering. */
    private fun pump() {
        if (app == 0L) return
        checkStatus(NativeApp.pollService(app))
        cancelRequests()
        val status = IntArray(1)
        repeat(MAX_PUMP_BATCH) {
            if (!jobs.canTakeRequest) {
                render()

                return
            }

            val request = NativeApp.nextRequest(app, status)
            checkStatus(status[0])

            if (status[0] == 0) {
                render()

                return
            }

            if (status[0] == REQUEST_DEFERRED) {
                render()
                schedulePump()

                return
            }

            jobs.launch(checkNotNull(request))
        }
        render()
        schedulePump()
    }

    private fun cancelRequests() {
        for (index in 0 until MAX_PUMP_BATCH) {
            val cancelled = NativeApp.nextCancelled(app)

            if (cancelled == 0L) break
            check(cancelled > 0) { "Invalid cancellation notification" }
            jobs.cancel(cancelled)

            if (index == MAX_PUMP_BATCH - 1) schedulePump()
        }
    }

    private fun schedulePump() {
        if (app == 0L || pumpScheduled) return
        pumpScheduled = true
        handler.post {
            pumpScheduled = false
            runUi { pump() }
        }
    }

    private fun event(
        node: Long,
        kind: Int,
        value: String = "",
    ) {
        if (app == 0L || renderer.applying) return
        runUi {
            checkStatus(NativeApp.event(app, node, kind, value.toByteArray(Charsets.UTF_8)))
            pump()
        }
    }

    private fun render() {
        renderer.render(NativeApp.renderBatch(app))
        riveAnimator?.requestFrame()

        if (!ready && !readyScheduled && childCount > 0) {
            readyScheduled = true
            post { reportReady() }
        }
    }

    private fun reportReady() {
        readyScheduled = false

        if (app != 0L && !ready && isAttachedToWindow && childCount > 0) {
            ready = true
            onReady()
        }
    }
}
