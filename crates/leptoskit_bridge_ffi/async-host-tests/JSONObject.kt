package org.json

/** The standalone JNI fixture must never enter Android rendering. */
class JSONObject(
    source: String,
) {
    init {
        error("Android rendering is unavailable in the JNI async fixture: $source")
    }

    fun getInt(name: String): Int = error("Android rendering is unavailable in the JNI async fixture: $name")
}
