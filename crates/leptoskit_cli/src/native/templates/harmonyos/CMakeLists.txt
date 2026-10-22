cmake_minimum_required(VERSION 3.16)
project(leptoskit_host LANGUAGES CXX)
# OHOS uses Clang's libc++; older SDKs incorrectly advertise a GCC toolchain.
unset(CMAKE_CXX_COMPILER_EXTERNAL_TOOLCHAIN)
# App sources live under hosts/harmonyos; generated inputs live at the app root.
include("${CMAKE_CURRENT_LIST_DIR}/../../../../../../.leptoskit/harmonyos/paths.cmake")
add_library(leptoskit SHARED "${LEPTOSKIT_BRIDGE}/harmonyos/bridge.cpp")
target_compile_features(leptoskit PRIVATE cxx_std_17)
target_compile_options(leptoskit PRIVATE -Wall -Wextra -Werror)
target_include_directories(leptoskit PRIVATE "${LEPTOSKIT_BRIDGE}/include")
target_link_libraries(leptoskit PRIVATE "${LEPTOSKIT_LIBRARY}" ace_napi.z hilog_ndk.z dl m)
