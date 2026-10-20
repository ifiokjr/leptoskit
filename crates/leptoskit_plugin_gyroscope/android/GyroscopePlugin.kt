package dev.leptoskit.plugins.gyroscope

import android.content.Context
import android.hardware.Sensor
import dev.leptoskit.sensors.MotionSensorPlugin

class GyroscopePlugin(
    context: Context,
) : MotionSensorPlugin(context, "gyroscope", Sensor.TYPE_GYROSCOPE)
