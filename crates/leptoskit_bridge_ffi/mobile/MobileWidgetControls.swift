import LeptosKitBridge
import UIKit

/// Native controls and keyboard gestures for the generated iOS renderer.
extension MobileWidgetRenderer: UITextFieldDelegate, UIGestureRecognizerDelegate {
    /// Press in and out follow the events on which UIKit sets and clears the
    /// button's highlight, so `active:` styles track it.
    func makeButton(_ id: Int) -> UIButton {
        let button = UIButton(type: .system)
        button.addAction(UIAction { [weak self] _ in self?.event(id, 0) }, for: .touchUpInside)
        button.addAction(
            UIAction { [weak self] _ in self?.event(id, Self.pressIn) }, for: [.touchDown, .touchDragEnter])
        button.addAction(
            UIAction { [weak self] _ in self?.event(id, Self.pressOut) },
            for: [.touchDragExit, .touchUpInside, .touchUpOutside, .touchCancel])

        return button
    }

    func makePressable(_ id: Int) -> UIView {
        let press = PressView()
        press.onHold = { [weak self] held in self?.event(id, held ? Self.pressIn : Self.pressOut) }
        press.isUserInteractionEnabled = true
        press.addGestureRecognizer(
            UITapGestureRecognizer(
                target: self, action: #selector(pressedWidget(_:))))

        return press
    }

    func makeTextField(_ id: Int, secure: Bool) -> UITextField {
        let field = UITextField()
        field.delegate = self
        if secure { field.maskEntry() }
        field.addAction(
            UIAction { [weak self, weak field] _ in
                self?.event(id, 1, field?.text ?? "")
            }, for: .editingChanged)

        return field
    }
    func makeControl(_ item: Change.Widget) throws -> UIView {
        let widget: UIView
        switch item.kind {
        case "switch", "Switch": widget = makeSwitch(item.id)
        case "checkbox", "Checkbox", "radio", "Radio":
            widget = makeChoice(item.id, radio: ["radio", "Radio"].contains(item.kind))
        case "slider", "Slider": widget = makeSlider(item.id)
        case "linear_progress_indicator", "LinearProgressIndicator": widget = LinearProgressView()
        case "activity_indicator", "ActivityIndicator": widget = UIActivityIndicatorView(style: .medium)
        default:
            throw NativeAppError(message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
        return widget
    }

    func makeSwitch(_ id: Int) -> UISwitch {
        let control = UISwitch()
        control.addAction(UIAction { [weak self, weak control] _ in
            guard let self, let control else { return }
            self.event(id, 6, "{\"checked\":\(control.isOn)}")
            control.isOn = self.properties[id]?.checked ?? false
        }, for: .valueChanged)
        return control
    }

    func makeChoice(_ id: Int, radio: Bool) -> UIButton {
        let button = ChoiceButton(type: .system)
        button.radio = radio
        button.addAction(UIAction { [weak self, weak button] _ in
            guard let self, let button else { return }
            let checked = radio || !(self.properties[id]?.checked ?? false)
            self.event(id, 6, "{\"checked\":\(checked)}")
            button.isSelected = self.properties[id]?.checked ?? false
        }, for: .touchUpInside)
        return button
    }

    func makeSlider(_ id: Int) -> UISlider {
        let slider = ValueSlider()
        slider.onStep = { [weak self, weak slider] increment in
            guard let self, let slider else { return }
            let state = self.properties[id]?.slider ?? .initial
            let step = state.step ?? (state.max - state.min) / 100
            let value = state.snapped(state.value + (increment ? step : -step))
            self.event(id, 7, "{\"value\":\(value)}")
            slider.value = (self.properties[id]?.slider ?? .initial).coordinate
        }
        slider.addAction(UIAction { [weak self, weak slider] _ in
            guard let self, let slider else { return }
            let state = self.properties[id]?.slider ?? .initial
            self.event(id, 7, "{\"value\":\(state.proposal(slider.value))}")
            slider.value = (self.properties[id]?.slider ?? .initial).coordinate
        }, for: .valueChanged)
        return slider
    }

    @objc func dismissKeyboard() { view.endEditing(true) }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        acceptsDismissal(from: touch.view)
    }

    /// Keep text selection and moving between fields under UIKit's control.
    func acceptsDismissal(from touched: UIView?) -> Bool {
        var ancestor = touched
        while let current = ancestor {
            if current is UITextField || current is UITextView { return false }
            ancestor = current.superview
        }
        return true
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool { true }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        // End editing before submitting so a handler can deliberately focus its
        // next field without having that focus dismissed afterwards.
        let text = textField.text ?? ""
        textField.resignFirstResponder()
        event(textField.tag, 2, text)

        return false
    }

    func textFieldDidBeginEditing(_ textField: UITextField) { event(textField.tag, 3) }
    func textFieldDidEndEditing(_ textField: UITextField) { event(textField.tag, 4) }
}
