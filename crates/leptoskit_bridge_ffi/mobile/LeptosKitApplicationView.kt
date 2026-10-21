package dev.leptoskit.mobile

import android.content.Context
import android.view.View
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import dev.leptoskit.bridge.NativeDispatcher
import org.json.JSONObject

/**
 * Native update controls and guest lifecycle. Runtime policy, signatures and
 * release selection stay inside Rust; only owned Wasm reaches the UI thread.
 */
class LeptosKitApplicationView(
    context: Context,
    private val wasm: ByteArray,
    embedded: ByteArray,
    configuration: ByteArray,
    private val register: (Long, NativeDispatcher) -> Unit,
) : LinearLayout(context),
    AutoCloseable {
    private val guests = FrameLayout(context)
    private val status = TextView(context)
    private val action = Button(context)
    private var releases: NativeReleases? = null
    private var current: LeptosKitView? = null
    private var pending: LeptosKitView? = null
    private var closed = false
    private var staged = false
    private var busy = false
    private var generation = 0L

    init {
        orientation = VERTICAL
        // Target SDK 35 draws edge to edge; keep the update bar and guest clear of system bars.
        fitsSystemWindows = true
        val enabled = JSONObject(configuration.toString(Charsets.UTF_8)).getBoolean("enabled")
        if (enabled) {
            status.contentDescription = "leptoskit-update-status"
            status.accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
            action.contentDescription = "leptoskit-update-action"
            action.isAllCaps = false
            val gutter = (16 * resources.displayMetrics.density).toInt()
            status.setPadding(gutter, gutter / 2, gutter, 0)
            action.text = "Check for update"
            action.setOnClickListener { if (staged) activate() else download() }
            addView(status, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
            addView(action, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
        }
        addView(guests, LayoutParams(LayoutParams.MATCH_PARENT, 0, 1f))
        if (enabled) {
            val owner = NativeReleases(context.noBackupFilesDir.resolve("leptoskit-updates"), embedded, configuration)
            releases = owner
            setBusy(true, "Loading application…")
            owner.open { result ->
                result.fold({ activate() }, { runBundledIfIdle("Cannot open updates", it) })
            }
        } else {
            current = LeptosKitView(context, wasm, register = register)
            guests.addView(current, FrameLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        }
    }

    private fun setBusy(
        value: Boolean,
        message: String,
    ) {
        busy = value
        status.text = message
        action.isEnabled = !value
        action.text = if (staged) "Activate update" else "Check for update"
    }

    /**
     * The update store failed before any guest ran. The store-shipped guest still
     * works without it, so run that rather than leave the window empty.
     */
    private fun runBundledIfIdle(
        prefix: String,
        error: Throwable,
    ) {
        if (closed || current != null || pending != null) {
            showError(prefix, error)
            return
        }
        showError(prefix, error)
        action.isEnabled = false
        try {
            val view = LeptosKitView(context, wasm, register = register)
            current = view
            guests.addView(view, FrameLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        } catch (failure: Exception) {
            showError("Cannot start application", failure)
        }
    }

    private fun showError(
        prefix: String,
        error: Throwable,
    ) {
        setBusy(false, "$prefix: ${error.message ?: error.javaClass.simpleName}")
    }

    private fun download() {
        if (closed || busy) return
        setBusy(true, "Checking for update…")
        releases?.sync { result ->
            result.fold(
                { report ->
                    staged = report.staged
                    val message =
                        if (staged) {
                            "Update ${report.version} (${report.build}) downloaded"
                        } else {
                            "Up to date"
                        }
                    setBusy(false, message)
                },
                { showError("Update failed", it) },
            )
        }
    }

    private fun activate() {
        if (closed || pending != null) return
        val owner = releases ?: return
        setBusy(true, "Starting application…")
        owner.launch { result ->
            result.fold({ mount(it) }, { runBundledIfIdle("Cannot launch application", it) })
        }
    }

    private fun mount(launch: NativeLaunch) {
        val token = ++generation
        try {
            val view =
                LeptosKitView(
                    context,
                    launch.wasm,
                    onReady = { confirm(launch, token) },
                    onFailure = { recover(launch, token, it) },
                    register = register,
                )
            pending = view
            if (current != null) {
                view.alpha = 0f
                view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            }
            guests.addView(view, 0, FrameLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        } catch (error: Exception) {
            recover(launch, token, error)
        }
    }

    private fun confirm(
        launch: NativeLaunch,
        token: Long,
    ) {
        if (closed || token != generation || pending == null) return
        releases?.confirm(launch) completion@{ result ->
            if (token != generation) return@completion
            result.fold(
                {
                    val view = pending ?: return@fold
                    pending = null
                    current?.let { old ->
                        guests.removeView(old)
                        old.close()
                    }
                    current = view
                    view.alpha = 1f
                    view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
                    view.bringToFront()
                    staged = false
                    setBusy(false, "Running ${launch.version} (${launch.build})")
                },
                { recover(launch, token, it) },
            )
        }
    }

    private fun recover(
        launch: NativeLaunch,
        token: Long,
        error: Throwable,
    ) {
        if (closed || token != generation) return
        ++generation
        pending?.let { view ->
            guests.removeView(view)
            view.close()
        }
        pending = null
        staged = false
        val owner = releases ?: return
        setBusy(true, "Recovering application…")
        owner.reject(launch) { result ->
            result.fold(
                {
                    // The confirmed view stays interactive throughout a candidate trial.
                    // A cold-start candidate needs one fresh fallback launch instead.
                    if (current == null && launch.candidate) {
                        activate()
                    } else {
                        showError("Update could not start", error)
                    }
                },
                { runBundledIfIdle("Cannot recover application", it) },
            )
        }
    }

    override fun close() {
        if (closed) return
        closed = true
        ++generation
        pending?.close()
        pending = null
        current?.close()
        current = null
        guests.removeAllViews()
        releases?.close()
        releases = null
    }
}
