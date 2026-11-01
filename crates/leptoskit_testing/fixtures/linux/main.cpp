#include <QApplication>
#include <QFileDialog>
#include <QFileInfo>
#include <QLabel>
#include <QLineEdit>
#include <QPushButton>
#include <QVBoxLayout>
#include <QWidget>

int main(int argc, char **argv) {
    QApplication app(argc, argv);
    QWidget window;
    window.setWindowTitle("Native capture lab");
    window.resize(560, 520);
    auto *layout = new QVBoxLayout(&window);
    auto *title = new QLabel("Native capture lab");
    title->setStyleSheet("font-size: 30px; font-weight: bold; color: #4a37a0;");
    layout->addWidget(title);

    layout->addWidget(new QLabel("Real text, native controls, system dialogs."));
    auto *name = new QLineEdit;
    name->setAccessibleName("name");
    name->setPlaceholderText("Your name");
    layout->addWidget(name);
    auto *status = new QLabel("Ready");
    status->setStyleSheet("font-size: 22px;");
    int count = 0;
    auto addButton = [&](const QString &text, const QString &id, auto action) {
        auto *button = new QPushButton(text);
        button->setAccessibleName(id);
        button->setMinimumHeight(42);
        QObject::connect(button, &QPushButton::clicked, action);
        layout->addWidget(button);
    };
    addButton("Greet", "greet", [&] { status->setText("Hello, " + name->text()); });
    addButton("Increment", "increment", [&] { status->setText("Count: " + QString::number(++count)); });
    addButton("Choose file", "permission", [&] {
        auto file = QFileDialog::getOpenFileName(&window, "Choose evidence", QApplication::applicationDirPath() + "/evidence.txt");
        status->setText(file.isEmpty() ? "Cancelled" : "Selected: " + QFileInfo(file).fileName());
    });

    layout->addWidget(status);
    window.show();

    return app.exec();
}
