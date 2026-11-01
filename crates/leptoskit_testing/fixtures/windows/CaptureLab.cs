using System;
using System.Drawing;
using System.IO;
using System.Windows.Forms;

public sealed class CaptureLab : Form {
    private readonly Label status = new Label { Name = "status", Text = "Ready", AutoSize = true, Font = new Font("Segoe UI", 14) };
    private int count;

    public CaptureLab() {
        Text = "Native capture lab";
        Size = new Size(560, 520);
        StartPosition = FormStartPosition.CenterScreen;
        BackColor = Color.FromArgb(245, 245, 252);
        var layout = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown, WrapContents = false, Padding = new Padding(24) };
        layout.Controls.Add(new Label { Text = "Native capture lab", AutoSize = true, ForeColor = Color.Indigo, Font = new Font("Segoe UI", 24, FontStyle.Bold) });
        layout.Controls.Add(new Label { Text = "Real text, native controls, system dialogs.", AutoSize = true });
        var name = new TextBox { Name = "name", Width = 450 };

        layout.Controls.Add(name);
        AddButton(layout, "Greet", "greet", () => status.Text = "Hello, " + name.Text);
        AddButton(layout, "Increment", "increment", () => status.Text = "Count: " + (++count));
        AddButton(layout, "Choose file", "permission", () => {
            using (var dialog = new OpenFileDialog { Title = "Choose evidence", FileName = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "evidence.txt"), CheckFileExists = true }) {
                status.Text = dialog.ShowDialog(this) == DialogResult.OK ? "Selected: " + Path.GetFileName(dialog.FileName) : "Cancelled";
            }
        });
        layout.Controls.Add(status);
        Controls.Add(layout);
    }

    private static void AddButton(Control parent, string text, string id, Action action) {
        var button = new Button { Text = text, Name = id, Width = 450, Height = 42, BackColor = Color.Lavender };
        button.Click += (sender, args) => action();
        parent.Controls.Add(button);
    }

    [STAThread]
    public static void Main() {
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new CaptureLab());
    }
}
