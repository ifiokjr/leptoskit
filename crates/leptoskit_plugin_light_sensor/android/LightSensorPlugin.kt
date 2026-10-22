package dev.leptoskit.plugins.lightsensor

import android.content.Context
import android.hardware.Sensor
import dev.leptoskit.sensors.MotionSensorPlugin

class LightSensorPlugin(
    context: Context,
) : MotionSensorPlugin(context, "light_sensor", Sensor.TYPE_LIGHT, bounded = true)
