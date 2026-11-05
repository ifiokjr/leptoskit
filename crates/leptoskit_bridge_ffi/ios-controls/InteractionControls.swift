import UIKit

/// Exercises the production renderer through wire transactions and real UIKit controls.
/// Touch and navigation gestures still need separate end-to-end simulator journeys.
extension UIKitControls {
    func interactionControls() throws {
        let host = UIView(frame: CGRect(x: 0, y: 0, width: 320, height: 640))
        window?.rootViewController?.view.addSubview(host)
        let renderer = MobileWidgetRenderer(view: host, devtools: nil)
        defer { renderer.close(); host.removeFromSuperview() }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        func render(_ mutations: String) throws {
            let json = "{\"version\":1,\"root\":1,\"mutations\":[\(mutations)]}"
            try renderer.render(decoder.decode(Transaction.self, from: Data(json.utf8)))
        }
        try render("""
        {"Create":{"id":1,"kind":"view","props":{}}},
        {"UpdateLayout":{"id":1,"x":0,"y":0,"width":320,"height":640}}
        """)
        let kinds = ["switch", "checkbox", "radio", "slider", "linear_progress_indicator",
                     "activity_indicator", "text_input", "password_input", "button"]
        for (index, kind) in kinds.enumerated() {
            let id = index + 2
            try render("""
            {"Create":{"id":\(id),"kind":"\(kind)","props":{"accessibility_label":"\(kind)"}}},
            {"InsertChild":{"parent":1,"child":\(id),"index":\(index)}},
            {"UpdateLayout":{"id":\(id),"x":0,"y":\(index * 50),"width":200,"height":44}}
            """)
        }
        let root = host.subviews[0]
        guard let toggle = root.subviews[0] as? UISwitch,
              let checkbox = root.subviews[1] as? UIButton,
              let radio = root.subviews[2] as? UIButton,
              let slider = root.subviews[3] as? UISlider,
              let progress = root.subviews[4] as? LinearProgressView,
              let activity = root.subviews[5] as? UIActivityIndicatorView,
              let field = root.subviews[6] as? UITextField,
              let password = root.subviews[7] as? UITextField,
              let button = root.subviews[8] as? UIButton else {
            record(false, "all generated iOS controls create native views")
            return
        }
        let controls = InteractionWidgets(toggle: toggle, checkbox: checkbox, radio: radio,
            slider: slider, progress: progress, activity: activity, field: field, password: password, button: button)
        try selectionControls(renderer, controls, render: render)
        try valueControls(renderer, controls, render: render)
        try keyboardControls(renderer, host: host, controls: controls, render: render)
    }

    private func selectionControls(
        _ renderer: MobileWidgetRenderer, _ controls: InteractionWidgets, render: @escaping (String) throws -> Void
    ) throws {
        let toggle = controls.toggle, checkbox = controls.checkbox, radio = controls.radio
        var events: [InteractionEvent] = []
        renderer.onEvent = { events.append(InteractionEvent(node: $0, kind: $1, text: $2)) }
        toggle.isOn = true
        toggle.sendActions(for: .valueChanged)
        record(events.last?.kind == 6 && events.last?.text == "{\"checked\":true}", "switch proposes toggle JSON")
        record(!toggle.isOn, "unaccepted switch proposal restores committed state")
        try render("""
        {"UpdateProps":{"id":2,"props":{"checked":true}}}
        """)
        toggle.isOn = false
        toggle.sendActions(for: .valueChanged)
        record(events.last?.text == "{\"checked\":false}" && toggle.isOn, "switch proposes both directions")
        checkbox.sendActions(for: .touchUpInside)
        record(events.last?.node == 3 && events.last?.text == "{\"checked\":true}", "checkbox selects")
        try render("""
        {"UpdateProps":{"id":3,"props":{"checked":true}}},
        {"UpdateProps":{"id":4,"props":{"checked":true}}}
        """)
        checkbox.sendActions(for: .touchUpInside)
        record(events.last?.text == "{\"checked\":false}", "checkbox unselects")
        radio.sendActions(for: .touchUpInside)
        record(events.last?.node == 4 && events.last?.text == "{\"checked\":true}",
            "selected radio never proposes false")
        record(radio.accessibilityTraits.contains(.selected) && radio.accessibilityValue == "1",
            "radio exposes selection to VoiceOver")
        var failure: Error?
        renderer.onEvent = { id, _, payload in
            do {
                try render("{\"UpdateProps\":{\"id\":\(id),\"props\":\(payload)}}")
            } catch { failure = error }
        }
        toggle.isOn = false
        toggle.sendActions(for: .valueChanged)
        record(!toggle.isOn && failure == nil, "accepted switch proposal retains synchronous guest state")
        checkbox.sendActions(for: .touchUpInside)
        record(!checkbox.isSelected && failure == nil, "accepted checkbox proposal retains synchronous guest state")
    }

    private func valueControls(
        _ renderer: MobileWidgetRenderer, _ controls: InteractionWidgets, render: @escaping (String) throws -> Void
    ) throws {
        let slider = controls.slider, toggle = controls.toggle, activity = controls.activity
        let progress = controls.progress, button = controls.button
        var events: [InteractionEvent] = []
        renderer.onEvent = { events.append(InteractionEvent(node: $0, kind: $1, text: $2)) }
        try render("""
        {"UpdateProps":{"id":5,"props":{"slider":{"min":10,"max":20,"value":12,"step":2}}}}
        """)
        slider.value = 0.57
        slider.sendActions(for: .valueChanged)
        record(events.last?.kind == 7 && events.last?.text == "{\"value\":16.0}", "slider emits snapped range value")
        record(abs(slider.value - 0.2) < 0.001, "unaccepted slider restores committed coordinate")
        slider.accessibilityIncrement()
        record(events.last?.text == "{\"value\":14.0}", "VoiceOver increments by the configured step")
        slider.accessibilityDecrement()
        record(events.last?.text == "{\"value\":10.0}", "VoiceOver decrements by the configured step")
        let before = events.count
        try render("""
        {"UpdateProps":{"id":2,"props":{"checked":false,"enabled":false}}},
        {"UpdateProps":{"id":6,"props":{"progress":{"determinate":0.75}}}},
        {"UpdateProps":{"id":7,"props":{"running":false}}}
        """)
        toggle.sendActions(for: .valueChanged)
        record(events.count == before && !toggle.isEnabled, "disabled controls and prop updates emit no proposals")
        record(progress.accessibilityValue == "75%" && !activity.isAnimating, "progress and activity apply state")
        try render("""
        {"UpdateProps":{"id":6,"props":{"progress":"indeterminate"}}},
        {"UpdateProps":{"id":7,"props":{"running":true}}}
        """)
        record(progress.accessibilityValue == "Loading" && activity.isAnimating, "indicators can restart")
        button.sendActions(for: .touchUpInside)
        record(events.contains { $0.node == 10 && $0.kind == 0 }, "toggle-style buttons retain press events")
    }

    private func keyboardControls(
        _ renderer: MobileWidgetRenderer, host: UIView, controls: InteractionWidgets,
        render: @escaping (String) throws -> Void
    ) throws {
        let field = controls.field, password = controls.password, button = controls.button
        var events: [InteractionEvent] = []
        renderer.onEvent = { events.append(InteractionEvent(node: $0, kind: $1, text: $2)) }
        record(password.isSecureTextEntry, "password remains masked")
        field.text = "hello"
        field.sendActions(for: .editingChanged)
        record(events.last?.kind == 1 && events.last?.text == "hello", "text input emits edits")
        record(field.becomeFirstResponder(), "text field receives focus")
        record(!renderer.acceptsDismissal(from: field) && renderer.acceptsDismissal(from: button),
            "outside tap filter preserves text selection and accepts button taps")
        record(host.gestureRecognizers?.first?.cancelsTouchesInView == false,
            "keyboard dismissal does not cancel control touches")
        renderer.dismissKeyboard()
        record(!field.isFirstResponder && events.last?.kind == 4, "outside dismissal ends editing and emits blur")
        _ = field.becomeFirstResponder()
        _ = renderer.textFieldShouldReturn(field)
        record(!field.isFirstResponder && events.last?.kind == 2 && events.last?.text == "hello",
            "return dismisses and submits the current input")
        try render("""
        {"UpdateProps":{"id":9,"props":{"focused":true}}}
        """)
        record(password.isFirstResponder, "guest can focus the next field after submit")
        renderer.dismissKeyboard()
        try render("""
        {"UpdateProps":{"id":9,"props":{"focused":true,"value":"changed"}}}
        """)
        record(!password.isFirstResponder, "dismissed field does not reclaim focus on an unrelated render")
        try render("""
        {"UpdateProps":{"id":9,"props":{"focused":false}}}
        """)
        try render("""
        {"UpdateProps":{"id":9,"props":{"focused":true}}}
        """)
        record(password.isFirstResponder, "a new guest focus request still takes effect")
        let count = events.count
        renderer.close()
        button.sendActions(for: .touchUpInside)
        record(events.count == count && host.gestureRecognizers?.isEmpty == true,
            "closed renderer releases dismissal and suppresses stale events")
    }
}

private struct InteractionEvent {
    let node: Int
    let kind: UInt32
    let text: String
}

private struct InteractionWidgets {
    let toggle: UISwitch
    let checkbox: UIButton
    let radio: UIButton
    let slider: UISlider
    let progress: LinearProgressView
    let activity: UIActivityIndicatorView
    let field: UITextField
    let password: UITextField
    let button: UIButton
}
