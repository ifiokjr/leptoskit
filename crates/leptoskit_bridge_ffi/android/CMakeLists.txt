cmake_minimum_required(VERSION 3.22)
project(leptoskit_bridge_jni LANGUAGES CXX)

# Pass the target-specific Rust static library built with cargo-ndk.
if(NOT DEFINED LEPTOSKIT_BRIDGE_LIBRARY)
    message(FATAL_ERROR "Set LEPTOSKIT_BRIDGE_LIBRARY to libleptoskit_bridge_ffi.a for this ABI")
endif()
add_library(leptoskit_bridge_rust STATIC IMPORTED)
set_target_properties(leptoskit_bridge_rust PROPERTIES IMPORTED_LOCATION "${LEPTOSKIT_BRIDGE_LIBRARY}")
add_library(leptoskit_bridge_jni SHARED bridge.cpp)
target_compile_features(leptoskit_bridge_jni PRIVATE cxx_std_20)
target_compile_options(leptoskit_bridge_jni PRIVATE -Wall -Wextra -Werror)
target_link_libraries(leptoskit_bridge_jni PRIVATE leptoskit_bridge_rust android log dl)
