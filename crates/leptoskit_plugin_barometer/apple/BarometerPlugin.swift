import LeptosKitSensors

@MainActor
public final class BarometerPlugin: MotionSensorPlugin {
    public init() { super.init(name: "barometer", kind: .barometer) }
}
