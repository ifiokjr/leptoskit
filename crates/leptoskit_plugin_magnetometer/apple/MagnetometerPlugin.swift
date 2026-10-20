import LeptosKitSensors

@MainActor
public final class MagnetometerPlugin: MotionSensorPlugin {
    public init() { super.init(name: "magnetometer", kind: .magnetometer) }
}
