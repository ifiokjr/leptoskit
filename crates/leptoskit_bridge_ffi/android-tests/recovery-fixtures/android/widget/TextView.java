package android.widget;
import android.content.Context;
import android.view.View;
public class TextView extends View {
    public CharSequence recordedText;
    public TextView(Context context) { super(context); }
    public void setText(CharSequence value) { recordedText = value; }

    public CharSequence getText() { return recordedText; }
}
