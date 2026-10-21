package dev.leptoskit.plugins.barometer

import android.content.Context
import android.hardware.Sensor
import dev.leptoskit.sensors.MotionSensorPlugin

class BarometerPlugin(
    context: Context,
) : MotionSensorPlugin(context, "barometer", Sensor.TYPE_PRESSURE)
