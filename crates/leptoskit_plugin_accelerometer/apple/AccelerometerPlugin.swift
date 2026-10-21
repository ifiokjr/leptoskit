import LeptosKitSensors

@MainActor
public final class AccelerometerPlugin: MotionSensorPlugin {
    public init() { super.init(name: "accelerometer", kind: .accelerometer) }
}
