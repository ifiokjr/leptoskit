package dev.leptoskit.plugins.magnetometer

import android.content.Context
import android.hardware.Sensor
import dev.leptoskit.sensors.MotionSensorPlugin

class MagnetometerPlugin(
    context: Context,
) : MotionSensorPlugin(context, "magnetometer", Sensor.TYPE_MAGNETIC_FIELD)
