package dev.leptoskit.capturelab;

import android.Manifest;
import android.app.Activity;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.os.Bundle;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;

public final class CaptureActivity extends Activity {
    private TextView status;
    private int count;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setPadding(32, 96, 32, 32);
        layout.setBackgroundColor(Color.rgb(245, 245, 252));
        TextView title = new TextView(this);
        title.setText("Native capture lab");
        title.setTextSize(30);
        title.setTextColor(Color.rgb(74, 55, 160));
        layout.addView(title);
        TextView subtitle = new TextView(this);
        subtitle.setText("Real text, native controls, system dialogs.");
        layout.addView(subtitle);
        EditText name = new EditText(this);
        name.setSingleLine(true);
        name.setHint("Your name");
        name.setContentDescription("name");
        layout.addView(name);
        status = new TextView(this);
        status.setContentDescription("status");
        status.setTextSize(22);
        status.setText("Ready");
        addButton(layout, "Greet", "greet", v -> status.setText("Hello, " + name.getText()));
        addButton(layout, "Increment", "increment", v -> status.setText("Count: " + (++count)));
        addButton(layout, "Enable notifications", "permission", v -> {
            if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) {
                status.setText("Notifications allowed");
            } else {
                requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 1);
            }
        });
        layout.addView(status);
        setContentView(layout);
    }

    private void addButton(LinearLayout layout, String label, String id, View.OnClickListener action) {
        Button button = new Button(this);
        button.setText(label);
        button.setContentDescription(id);
        button.setOnClickListener(action);
        layout.addView(button);
    }

    @Override public void onRequestPermissionsResult(int request, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(request, permissions, results);
        if (request == 1) status.setText(results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED
            ? "Notifications allowed" : "Notifications denied");
    }
}
