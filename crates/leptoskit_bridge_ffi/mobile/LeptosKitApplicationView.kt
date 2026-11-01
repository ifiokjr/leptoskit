package dev.leptoskit.mobile

import android.app.Activity
import android.content.Context
import android.graphics.Color
import android.os.Build
import android.os.Looper
import android.util.Log
import android.view.View
import android.view.Window
import android.view.WindowInsetsController
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import dev.leptoskit.bridge.NativeBridge
import dev.leptoskit.bridge.NativeDispatcher
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import org.json.JSONObject

/**
 * Native update controls and guest lifecycle. Runtime policy, signatures and
 * release selection stay inside Rust; only owned Wasm reaches the UI thread.
 */
class LeptosKitApplicationView(
    context: Context,
    private val embedded: ByteArray,
    configuration: ByteArray,
    private val register: (Long, NativeDispatcher) -> Unit,
) : LinearLayout(context),
    AutoCloseable {
    private companion object {
        const val UPDATE_GUTTER_DP = 16
        const val LIGHT_LUMINANCE = 0.5f
    }

    private val guests = FrameLayout(context)
    private val status = TextView(context)
    private val action = Button(context)
    private var releases: NativeReleases? = null
    private var development: NativeDevelopmentGuests? = null
    private var current: LeptosKitView? = null
        set(value) {
            field = value
            paintCanvas()
        }
    private val window = (context as? Activity)?.window

    @Suppress("DEPRECATION")
    private val themeBars = window?.let { SystemBars(it.statusBarColor, it.navigationBarColor) }
    private var pending: LeptosKitView? = null
    private val retiring = mutableSetOf<LeptosKitView>()
    private var closed = false
    private var staged = false
    private var busy = false
    private var generation = 0L
    private var recoveryFailure: Throwable? = null
    private val lifecycle = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val closeCompletion = CompletableDeferred<Unit>()
    private val transitions = mutableSetOf<Job>()

    private fun transition(block: suspend () -> Unit) {
        val task = lifecycle.launch { block() }
        transitions.add(task)
        task.invokeOnCompletion { transitions.remove(task) }
    }

    init {
        orientation = VERTICAL
        // Target SDK 35 draws edge to edge; keep the update bar and guest clear of system bars.
        fitsSystemWindows = true
        val developmentUrl = NativeDevelopmentGuests.url(context)
        // `leptoskit dev` serves every new guest itself, so it replaces the release store.
        val enabled = developmentUrl == null && JSONObject(configuration.toString(Charsets.UTF_8)).getBoolean("enabled")

        if (enabled) {
            status.contentDescription = "leptoskit-update-status"
            status.accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
            action.contentDescription = "leptoskit-update-action"
            action.isAllCaps = false
            val gutter = (UPDATE_GUTTER_DP * resources.displayMetrics.density).toInt()
            status.setPadding(gutter, gutter / 2, gutter, 0)
            action.text = "Check for update"

            action.setOnClickListener { if (staged) activate() else download() }

            // The update bar keeps the light theme it was designed for whatever
            // colour the guest paints the canvas.
            val bar = LinearLayout(context)
            bar.orientation = VERTICAL
            bar.setBackgroundColor(Color.WHITE)
            bar.addView(status, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
            bar.addView(action, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
            addView(bar, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
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
            NativeBridge.load()
            // Before the first app, so every guest `leptoskit dev` sends reports to its hub.
            NativeDevtools.configure(context)
            val wasm = NativeUpdates.entry(embedded)

            current = adopt(LeptosKitView(context, wasm, register = register))
            guests.addView(current, FrameLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
            development = developmentUrl?.let { NativeDevelopmentGuests(it, wasm, ::reload) }
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
            NativeBridge.load()
            val view = adopt(LeptosKitView(context, NativeUpdates.entry(embedded), register = register))
            current = view

            guests.addView(view, FrameLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        } catch (failure: Exception) {
            showError("Cannot start application", NativeApplicationTeardown.retain(error, failure))
        }
    }

    private fun showError(
        prefix: String,
        error: Throwable,
    ) {
        setBusy(false, "$prefix: ${NativeApplicationTeardown.message(error)}")
    }

    private fun download() {
        if (closed || busy) return
        recoveryFailure = null
        setBusy(true, "Checking for update…")
        releases?.sync { result ->
            result.fold(
                { report ->
                    staged = report.staged
                    val message = updateMessage(report)
                    setBusy(false, message)
                },
                { showError("Update failed", it) },
            )
        }
    }

    private fun updateMessage(report: NativeSync): String {
        if (staged) return "Update ${report.version} (${report.build}) downloaded"
        return "Up to date"
    }

    private fun activate() {
        if (closed || pending != null) return
        val owner = releases ?: return
        setBusy(true, "Starting application…")
        owner.launch { result ->
            result.fold({ mount(it) }, { runBundledIfIdle("Cannot launch application", it) })
        }
    }

    /** Keep a candidate hidden until first render confirms it, joining failed construction. */
    private fun mount(launch: NativeLaunch) {
        if (closed) return
        val token = ++generation
        try {
            val view =
                adopt(
                    LeptosKitView(
                        context,
                        launch.wasm,
                        onReady = { confirm(launch, token) },
                        onFailure = { recover(launch, token, it) },
                        register = register,
                    ),
                )
            pending = view

            if (current != null) {
                view.alpha = 0f

                view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            }

            guests.addView(view, 0, FrameLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        } catch (error: Exception) {
            transition { recoverInitialization(launch, token, error) }
        }
    }

    private suspend fun recoverInitialization(
        launch: NativeLaunch,
        token: Long,
        error: Exception,
    ) {
        if (error is NativeApplicationInitializationFailure) joinInitialization(error)

        recover(launch, token, error)
    }

    private suspend fun joinInitialization(error: NativeApplicationInitializationFailure) {
        NativeApplicationTeardown.retain(error, NativeApplicationTeardown.join { error.join() })
    }

    private fun confirm(
        launch: NativeLaunch,
        token: Long,
    ) {
        if (closed || token != generation || pending == null) return
        val view = pending ?: return
        releases?.confirm(launch) completion@{ result ->

            if (closed || token != generation || pending !== view) return@completion
            result.fold(
                { transition { promote(launch, token, view) } },
                { recover(launch, token, it) },
            )
        }
    }

    private suspend fun promote(
        launch: NativeLaunch,
        token: Long,
        view: LeptosKitView,
    ) {
        if (closed || token != generation || pending !== view) return
        val previous = current
        previous?.let { retiring.add(it) }
        val cleanup = NativeApplicationTeardown.join(previous)
        retire(previous)

        if (closed || token != generation || pending !== view) return
        expose(launch, view, cleanup)
    }

    private fun adopt(view: LeptosKitView): LeptosKitView {
        view.onCanvasChanged = { if (it === current) paintCanvas() }

        return view
    }

    /**
     * Paint the current guest's root background behind the system bars, as a
     * browser paints the root element's background across the whole canvas,
     * with bar icons that contrast with it. Without one, keep the theme's.
     */
    private fun paintCanvas() {
        val color = current?.canvas

        if (color == null) background = null else setBackgroundColor(color)

        // The window has no decor, and so no bars, until this view is attached.
        if (!isAttachedToWindow) return
        val window = window ?: return
        val bars = themeBars ?: return
        val light = color == null || Color.luminance(color) > LIGHT_LUMINANCE
        window.useLightSystemBars(light)

        // Before Android 15's enforced edge-to-edge, the bars draw their own colour.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) {
            @Suppress("DEPRECATION")
            window.statusBarColor = color ?: bars.status
            @Suppress("DEPRECATION")
            window.navigationBarColor = color ?: bars.navigation
        }
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        paintCanvas()
    }

    private fun retire(view: LeptosKitView?) {
        view?.let { guests.removeView(it) }
        retiring.remove(view)

        if (current === view) current = null
    }

    private fun expose(
        launch: NativeLaunch,
        view: LeptosKitView,
        cleanup: Exception?,
    ) {
        pending = null
        current = view
        view.alpha = 1f
        view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_AUTO

        view.bringToFront()
        staged = false
        val warning = recoveryFailure?.let { NativeApplicationTeardown.retain(it, cleanup) } ?: cleanup
        recoveryFailure = null
        val running = "Running ${launch.version} (${launch.build})"

        if (warning == null) setBusy(false, running) else showError("$running; cleanup reported", warning)
    }

    private fun recover(
        launch: NativeLaunch,
        token: Long,
        error: Throwable,
    ) {
        if (closed || token != generation) return
        val recovery = ++generation
        val failed = pending
        failed?.let { retiring.add(it) }
        pending = null
        staged = false

        val owner = releases ?: return

        setBusy(true, "Recovering application…")
        transition { rejectAfterJoin(owner, launch, recovery, failed, error) }
    }

    /** Reject only after the failed guest joins; generation guards every delayed callback. */
    private suspend fun rejectAfterJoin(
        owner: NativeReleases,
        launch: NativeLaunch,
        token: Long,
        failed: LeptosKitView?,
        error: Throwable,
    ) {
        val cleanup = NativeApplicationTeardown.join(failed)
        retire(failed)

        if (closed || token != generation) return
        val diagnostic = NativeApplicationTeardown.retain(error, cleanup)
        recoveryFailure = diagnostic
        try {
            owner.reject(launch) { result -> reportRejection(result, launch, token, error) }
        } catch (failure: Exception) {
            if (!closed && token == generation) {
                runBundledIfIdle("Cannot recover application", NativeApplicationTeardown.retain(diagnostic, failure))
            }
        }
    }

    private fun reportRejection(
        result: Result<Unit>,
        launch: NativeLaunch,
        token: Long,
        error: Throwable,
    ) {
        if (closed || token != generation) return
        result.fold(
            { if (current == null && launch.candidate) activate() else showError("Update could not start", error) },
            { runBundledIfIdle("Cannot recover application", NativeApplicationTeardown.retain(error, it)) },
        )
    }

    /**
     * Mounts a guest from `leptoskit dev` hidden behind the running one and swaps
     * them once it renders, so a reload never shows an empty screen. A newer guest
     * replaces a candidate that has not rendered yet.
     */
    private fun reload(wasm: ByteArray) {
        if (closed) return
        val token = ++generation
        pending?.let { stale ->
            pending = null
            retiring.add(stale)
            transition {
                NativeApplicationTeardown.join(stale)
                retire(stale)
            }
        }
        var candidate: LeptosKitView? = null
        try {
            val view =
                adopt(
                    LeptosKitView(
                        context,
                        wasm,
                        onReady = { transition { promoteDevelopment(token) } },
                        onFailure = { developmentFailed(candidate, it) },
                        register = register,
                    ),
                )
            candidate = view
            pending = view

            view.alpha = 0f
            view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            guests.addView(view, 0, FrameLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        } catch (error: Exception) {
            reportDevelopment(error)

            if (error is NativeApplicationInitializationFailure) transition { joinInitialization(error) }
        }
    }

    private suspend fun promoteDevelopment(token: Long) {
        if (closed || token != generation) return
        val view = pending ?: return
        val previous = current
        previous?.let { retiring.add(it) }
        val cleanup = NativeApplicationTeardown.join(previous)
        retire(previous)

        if (closed || token != generation || pending !== view) return
        pending = null
        current = view
        view.alpha = 1f
        view.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
        view.bringToFront()
        Log.i("LeptosKit", "leptoskit dev: reloaded")
        cleanup?.let { Log.w("LeptosKit", "leptoskit dev: previous guest cleanup: ${NativeApplicationTeardown.message(it)}") }
    }

    /** A candidate that fails keeps the running guest; a running guest that fails shows why. */
    private fun developmentFailed(
        view: LeptosKitView?,
        error: Exception,
    ) {
        reportDevelopment(error)

        if (closed || view == null) return

        if (view === pending) {
            pending = null
            retiring.add(view)
            transition {
                NativeApplicationTeardown.join(view)
                retire(view)
            }
        } else if (view === current) {
            view.addView(
                TextView(context).apply {
                    text = NativeApplicationTeardown.message(error)
                    contentDescription = "leptoskit-error"
                },
            )
        }
    }

    private fun reportDevelopment(error: Throwable) {
        val message = "leptoskit dev: ${NativeApplicationTeardown.message(error)}"
        Log.w("LeptosKit", message)
        Toast.makeText(context, message, Toast.LENGTH_LONG).show()
    }

    override fun close() {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Application close requires the main thread" }

        if (closed) return
        closed = true
        development?.close()
        development = null
        ++generation
        val views = (listOfNotNull(pending, current) + retiring).distinct()

        for (view in views) view.close()
        pending = null
        current = null
        guests.removeAllViews()
        val owner = releases

        releases = null
        val joining = transitions.toList()
        lifecycle.launch { finishClose(views, joining, owner) }
    }

    private suspend fun finishClose(
        views: List<LeptosKitView>,
        joining: List<Job>,
        owner: NativeReleases?,
    ) {
        var failure: Exception? = null
        try {
            failure = joinViews(views)
            joining.joinAll()
        } finally {
            owner?.close()

            if (failure == null) closeCompletion.complete(Unit) else closeCompletion.completeExceptionally(failure)
            lifecycle.cancel()
        }
    }

    /** Join every retained guest and preserve all failures for the unload caller. */
    private suspend fun joinViews(views: List<LeptosKitView>): Exception? {
        var failure: Exception? = null

        for (view in views) failure = joinView(view, failure)

        return failure
    }

    private suspend fun joinView(
        view: LeptosKitView,
        previous: Exception?,
    ): Exception? {
        try {
            view.closeAndJoin()
        } catch (error: Exception) {
            if (previous == null) return error
            previous.addSuppressed(error)
        }

        return previous
    }

    /** Returns after guest services and native plugin jobs have joined. */
    suspend fun closeAndJoin() {
        close()
        closeCompletion.await()
    }
}

private class SystemBars(
    val status: Int,
    val navigation: Int,
)

/** Dark icons on light bars, light icons on dark ones. */
private fun Window.useLightSystemBars(light: Boolean) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        val mask =
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or
                WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS

        insetsController?.setSystemBarsAppearance(if (light) mask else 0, mask)

        return
    }

    @Suppress("DEPRECATION")
    val flags = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
    @Suppress("DEPRECATION")
    decorView.systemUiVisibility =
        if (light) decorView.systemUiVisibility or flags else decorView.systemUiVisibility and flags.inv()
}
