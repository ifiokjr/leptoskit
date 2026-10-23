plugins { id("com.android.application") version "9.0.1" apply false }

allprojects {
    layout.buildDirectory.set(rootProject.layout.projectDirectory.dir("target/$name"))
}
