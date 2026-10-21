package dev.leptoskit.plugins.accelerometer

import android.content.Context
import android.hardware.Sensor
import dev.leptoskit.sensors.MotionSensorPlugin

class AccelerometerPlugin(
    context: Context,
) : MotionSensorPlugin(context, "accelerometer", Sensor.TYPE_ACCELEROMETER)
