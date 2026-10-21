import LeptosKitSensors

@MainActor
public final class GyroscopePlugin: MotionSensorPlugin {
    public init() { super.init(name: "gyroscope", kind: .gyroscope) }
}
