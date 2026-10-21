package dev.leptoskit.gallery

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.os.Bundle
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.ViewTreeObserver
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.EditText
import android.widget.ScrollView
import android.widget.TextView
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Exercises real Activity/View input and captures every shared screen. */
class GalleryInstrumentation : Instrumentation() {
    private fun onMain(action: () -> Unit) {
        var failure: Throwable? = null
        runOnMainSync {
            try {
                action()
            } catch (error: Throwable) {
                failure = error
            }
        }
        failure?.let { throw it }
    }

    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        start()
    }

    override fun onStart() {
        val report = Bundle()

        try {
            for (screen in listOf("counter", "profile", "settings", "photos", "chat", "dashboard", "components")) {
                val intent =
                    Intent(targetContext, GalleryActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                        .putExtra("screen", screen)
                val activity = startActivitySync(intent)
                painted(activity)

                if (screen == "counter") verifyCounter(activity)
                if (screen == "components") verifyStudio(activity)

                onMain {
                    val root = activity.findViewById<View>(android.R.id.content)
                    check(root.width > 0 && root.height > 0) { "$screen did not receive a viewport" }
                    val image = Bitmap.createBitmap(root.width, root.height, Bitmap.Config.ARGB_8888)
                    root.draw(Canvas(image))
                    File(activity.filesDir, "gallery-$screen.png").outputStream().use {
                        check(image.compress(Bitmap.CompressFormat.PNG, 100, it))
                    }
                    image.recycle()
                    activity.finish()
                }
                waitForIdleSync()
            }

            report.putString(
                "result",
                "VERIFIED: seven shared screens, native pointer input, resize, retained control identity, and Activity teardown",
            )
            finish(Activity.RESULT_OK, report)
        } catch (error: Throwable) {
            report.putString("result", "NOT VERIFIED: ${error.stackTraceToString()}")
            finish(Activity.RESULT_CANCELED, report)
        }
    }

    private fun painted(activity: Activity) {
        waitForIdleSync()
        val drawn = CountDownLatch(1)
        onMain {
            val view = activity.window.decorView
            val listener =
                object : ViewTreeObserver.OnDrawListener {
                    override fun onDraw() {
                        view.post { view.viewTreeObserver.removeOnDrawListener(this) }
                        drawn.countDown()
                    }
                }
            view.viewTreeObserver.addOnDrawListener(listener)
            view.postInvalidateOnAnimation()
        }
        check(drawn.await(5, TimeUnit.SECONDS)) { "The gallery did not draw within five seconds" }
        waitForIdleSync()
    }

    private fun views(activity: Activity): List<View> {
        val result = mutableListOf<View>()

        fun visit(view: View) {
            result.add(view)
            if (view is ViewGroup) repeat(view.childCount) { visit(view.getChildAt(it)) }
        }
        visit(activity.findViewById(android.R.id.content))
        return result
    }

    private fun verifyCounter(activity: Activity) {
        resize(activity, false)
        lateinit var button: Button
        onMain {
            button = views(activity).filterIsInstance<Button>().single { it.text.toString() == "Add one" }
            check(button.isShown && button.width > 0 && button.height > 0) {
                "Add one was not visible: " +
                    views(activity).joinToString { "${it.javaClass.simpleName} ${it.width}x${it.height} shown=${it.isShown}" }
            }
        }
        tap(button)
        painted(activity)
        onMain {
            check(views(activity).filterIsInstance<TextView>().any { it.text.toString() == "13" }) {
                "Native pointer input did not reconcile the shared counter"
            }
        }
        resize(activity, true)
        onMain {
            check(views(activity).any { it === button }) { "Rotation replaced the live native button" }
            check(views(activity).filterIsInstance<TextView>().any { it.text.toString() == "13" }) {
                "Rotation lost application-owned counter state"
            }
        }
        resize(activity, false)
    }

    private fun tap(view: View) {
        val location = IntArray(2)
        onMain {
            check(view.isShown && view.width > 0 && view.height > 0)
            view.getLocationOnScreen(location)
            location[0] += view.width / 2
            location[1] += view.height / 2
        }
        val now = SystemClock.uptimeMillis()
        val down = MotionEvent.obtain(now, now, MotionEvent.ACTION_DOWN, location[0].toFloat(), location[1].toFloat(), 0)
        val up = MotionEvent.obtain(now, now + 50, MotionEvent.ACTION_UP, location[0].toFloat(), location[1].toFloat(), 0)

        try {
            sendPointerSync(down)
            sendPointerSync(up)
        } finally {
            down.recycle()
            up.recycle()
        }
    }

    private fun verifyStudio(activity: Activity) {
        resize(activity, false)
        lateinit var root: ScrollView
        lateinit var editor: EditText
        lateinit var action: View
        lateinit var scrollListener: ViewTreeObserver.OnScrollChangedListener
        val scrolled = CountDownLatch(1)
        onMain {
            root = views(activity).filterIsInstance<ScrollView>().first()
            editor = views(activity).filterIsInstance<EditText>().single { it.contentDescription == "Demo password" }
            action = views(activity).single { it.contentDescription == "Add a preview" }
            check(editor.width / activity.resources.displayMetrics.density > 250f) { "Phone studio input is too narrow" }
            editor.setText("Gallery value")
            check(editor.text.toString() == "Gallery value")
            check(root.canScrollVertically(1)) { "Phone studio has no vertical document overflow" }
            scrollListener =
                ViewTreeObserver.OnScrollChangedListener {
                    if (root.scrollY > 0) scrolled.countDown()
                }
            root.viewTreeObserver.addOnScrollChangedListener(scrollListener)
            check(root.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, null)) {
                "Native accessibility could not scroll the phone studio"
            }
        }
        check(scrolled.await(5, TimeUnit.SECONDS)) { "The phone studio did not scroll" }
        settled(root)
        painted(activity)
        onMain {
            root.viewTreeObserver.removeOnScrollChangedListener(scrollListener)
            action.requestRectangleOnScreen(Rect(0, 0, action.width, action.height), true)
        }
        painted(activity)
        tap(action)
        painted(activity)
        onMain {
            check(views(activity).filterIsInstance<TextView>().any { it.text.toString() == "13 previews created" }) {
                "The scrolled native Pressable did not reconcile shared studio state"
            }
            val content = activity.findViewById<View>(android.R.id.content)
            val image = Bitmap.createBitmap(content.width, content.height, Bitmap.Config.ARGB_8888)
            content.draw(Canvas(image))
            File(activity.filesDir, "gallery-components-scrolled.png").outputStream().use {
                check(image.compress(Bitmap.CompressFormat.PNG, 100, it))
            }
            image.recycle()
        }
        resize(activity, true)
        onMain {
            check(views(activity).any { it === editor }) { "Phone rotation replaced the native studio editor" }
            check(editor.text.toString() == "Gallery value") { "Phone rotation lost the controlled editor value" }
        }
        resize(activity, false)
        onMain { root.scrollTo(0, 0) }
        painted(activity)
    }

    private fun settled(view: View) {
        val idle = CountDownLatch(1)
        onMain {
            view.postOnAnimation(
                object : Runnable {
                    private var previous = Int.MIN_VALUE
                    private var unchanged = 0

                    override fun run() {
                        unchanged = if (view.scrollY == previous) unchanged + 1 else 0
                        previous = view.scrollY

                        if (unchanged >= 3) idle.countDown() else view.postOnAnimation(this)
                    }
                },
            )
        }
        check(idle.await(5, TimeUnit.SECONDS)) { "Native scrolling did not settle" }
    }

    private fun resize(
        activity: Activity,
        landscape: Boolean,
    ) {
        val resized = CountDownLatch(1)
        onMain {
            val content = activity.findViewById<View>(android.R.id.content)
            lateinit var listener: View.OnLayoutChangeListener

            fun matches(): Boolean = content.width > 0 && content.height > 0 && (content.width > content.height) == landscape

            listener =
                View.OnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
                    if (matches()) {
                        content.removeOnLayoutChangeListener(listener)
                        resized.countDown()
                    }
                }
            content.addOnLayoutChangeListener(listener)
            activity.requestedOrientation =
                if (landscape) ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE else ActivityInfo.SCREEN_ORIENTATION_PORTRAIT

            if (matches()) {
                content.removeOnLayoutChangeListener(listener)
                resized.countDown()
            }
        }
        check(resized.await(5, TimeUnit.SECONDS)) { "The native gallery viewport did not rotate" }
        painted(activity)
    }
}
