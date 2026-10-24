package android.widget;
import android.content.Context;
import android.view.View;
import android.view.ViewGroup;
import java.util.ArrayList;
import java.util.List;
public class FrameLayout extends ViewGroup {
    public static final List<View> removed = new ArrayList<>();
    public FrameLayout(Context context) { super(context); }
    @Override protected void onLayout(boolean changed, int left, int top, int right, int bottom) {}
    @Override public void removeView(View view) { removed.add(view); }
    @Override public void removeAllViews() { removed.clear(); }
    @Override public void setAlpha(float value) {}
    @Override public void setImportantForAccessibility(int value) {}
    @Override public void bringToFront() {}
}
