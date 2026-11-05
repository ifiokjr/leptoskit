import AppKit
import LeptosKitBridge

final class MacChoiceButton: NSButton {
    var radio = false
}

/// Keyboard and accessibility increments use the guest's step grid.
final class MacValueSlider: NSSlider {
    var state = WidgetSlider.initial

    private func step(_ forward: Bool) -> Bool {
        guard isEnabled else { return false }
        let delta = state.step ?? (state.max - state.min) / 100
        doubleValue = state.snapped(state.value + (forward ? delta : -delta))
        return sendAction(action, to: target)
    }

    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 123, 125: _ = step(false)
        case 124, 126: _ = step(true)
        default: super.keyDown(with: event)
        }
    }

    override func accessibilityPerformIncrement() -> Bool { step(true) }
    override func accessibilityPerformDecrement() -> Bool { step(false) }
}

extension MacWidgetRenderer {
    func makeControl(_ item: Change.Widget) throws -> NSView {
        switch item.kind {
        case "switch", "Switch":
            let control = NSSwitch()
            control.target = self
            control.action = #selector(toggled(_:))
            return control
        case "checkbox", "Checkbox", "radio", "Radio":
            let control = MacChoiceButton()
            control.radio = ["radio", "Radio"].contains(item.kind)
            control.setButtonType(control.radio ? .radio : .switch)
            control.target = self
            control.action = #selector(toggled(_:))
            return control
        case "slider", "Slider":
            let control = MacValueSlider(value: 0, minValue: 0, maxValue: 1, target: self, action: #selector(slid(_:)))
            control.isContinuous = true
            return control
        case "linear_progress_indicator", "LinearProgressIndicator", "activity_indicator", "ActivityIndicator":
            let control = NSProgressIndicator()
            control.style = ["activity_indicator", "ActivityIndicator"].contains(item.kind) ? .spinning : .bar
            return control
        default:
            throw NativeAppError(message: "Unsupported native widget: \(item.kind ?? "missing kind")")
        }
    }

    @objc private func toggled(_ control: NSControl) {
        guard let id = node(control) else { return }
        let checked: Bool
        if let button = control as? MacChoiceButton {
            checked = button.radio || button.state == .on
        } else {
            checked = (control as? NSSwitch)?.state == .on
        }
        event(id, 6, "{\"checked\":\(checked)}")
        // AppKit can deselect sibling radios before invoking the target. Restore
        // every control from the guest's synchronous response, even on rejection.
        for (id, widget) in widgets {
            if let props = properties[id] { MacControlStyle.apply(props, to: widget) }
        }
    }

    @objc private func slid(_ control: NSSlider) {
        guard let id = node(control) else { return }
        let state = properties[id]?.slider ?? .initial
        event(id, 7, "{\"value\":\(state.snapped(control.doubleValue))}")
        if let props = properties[id] { MacControlStyle.apply(props, to: control) }
    }
}

@MainActor
enum MacControlStyle {
    static func apply(_ props: WidgetProps, to widget: NSView) {
        if let control = widget as? NSControl { control.isEnabled = props.enabled ?? true }
        if let control = widget as? NSSwitch { control.state = props.checked == true ? .on : .off }
        if let control = widget as? MacChoiceButton {
            control.state = props.checked == true ? .on : .off
            control.isBordered = false
        }
        if let control = widget as? NSSlider {
            let state = props.slider ?? .initial
            (control as? MacValueSlider)?.state = state
            control.minValue = state.min
            control.maxValue = state.max
            control.doubleValue = state.value
            control.setAccessibilityValue(NSNumber(value: (props.slider ?? .initial).value))
        }
        if let control = widget as? NSProgressIndicator {
            if control.style == .spinning {
                control.isIndeterminate = true
                if props.running ?? true { control.startAnimation(nil) } else { control.stopAnimation(nil) }
            } else {
                switch props.progress ?? .indeterminate {
                case .indeterminate:
                    control.isIndeterminate = true
                    control.startAnimation(nil)
                case .determinate(let value):
                    control.stopAnimation(nil)
                    control.isIndeterminate = false
                    control.minValue = 0
                    control.maxValue = 1
                    control.doubleValue = Double(value)
                }
            }
        }
    }
}
