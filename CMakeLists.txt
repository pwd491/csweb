cmake_minimum_required(VERSION 3.22)


if(ANDROID)
    if(NOT ANDROID_ABI MATCHES "^(arm64-v8a|armeabi-v7a)$")
        message(WARNING "CSWeb is not supported on this arch, skipping")
        return()
    endif()
elseif(CMAKE_SIZEOF_VOID_P EQUAL 4)
    message(WARNING "CSWeb is not supported on this arch, skipping")
    return()
endif()

include(FetchContent)
FetchContent_Declare(Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG        v0.6.1)
FetchContent_MakeAvailable(Corrosion)

set(_saved_lib_out "${CMAKE_LIBRARY_OUTPUT_DIRECTORY}")
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/rust-out")

corrosion_import_crate(
    MANIFEST_PATH "${CMAKE_CURRENT_SOURCE_DIR}/rust/Cargo.toml"
    CRATES cswebservo
    PROFILE release)

set(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${_saved_lib_out}")

set(_prefix "")
if(ANDROID)
    set(_prefix "lib")
endif()
set(_file "${_prefix}csweb${POSTFIX}${CMAKE_SHARED_LIBRARY_SUFFIX}")

if(ANDROID AND CMAKE_LIBRARY_OUTPUT_DIRECTORY)
    set(_out "${CMAKE_LIBRARY_OUTPUT_DIRECTORY}")
else()
    set(_out "${CMAKE_BINARY_DIR}/cl_dll")
endif()

add_custom_target(csweb ALL
    COMMAND ${CMAKE_COMMAND} -E make_directory "${_out}"
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "$<TARGET_FILE:cswebservo-shared>" "${_out}/${_file}"
    VERBATIM)
add_dependencies(csweb cswebservo-shared)

# libcsweb_android.so needed libc++_shared.so in runtime
# so copy it from NDK
if(ANDROID AND NOT ANDROID_STL STREQUAL "c++_shared")
    if(ANDROID_ABI STREQUAL "arm64-v8a")
        set(_ndk_triple "aarch64-linux-android")
    elseif(ANDROID_ABI STREQUAL "armeabi-v7a")
        set(_ndk_triple "arm-linux-androideabi")
    endif()

    file(GLOB _libcxx
        "${CMAKE_ANDROID_NDK}/toolchains/llvm/prebuilt/*/sysroot/usr/lib/${_ndk_triple}/libc++_shared.so")
    if(NOT _libcxx)
        message(FATAL_ERROR "libc++_shared.so not found: NDK=${CMAKE_ANDROID_NDK} triple=${_ndk_triple}")
    endif()
    list(GET _libcxx 0 _libcxx)

    add_custom_target(csweb_libcxx ALL
        COMMAND ${CMAKE_COMMAND} -E make_directory "${_out}"
        COMMAND ${CMAKE_COMMAND} -E copy_if_different "${_libcxx}" "${_out}/libc++_shared.so"
        VERBATIM)
    add_dependencies(csweb csweb_libcxx)
endif()

install(FILES "$<TARGET_FILE:cswebservo-shared>"
        RENAME      "${_file}"
        DESTINATION "${GAME_DIR}/${CLIENT_INSTALL_DIR}"
        PERMISSIONS OWNER_READ OWNER_WRITE OWNER_EXECUTE
                    GROUP_READ GROUP_EXECUTE WORLD_READ WORLD_EXECUTE)

add_library(csweb_client INTERFACE)
add_library(csweb::client ALIAS csweb_client)
target_include_directories(csweb_client INTERFACE "${CMAKE_CURRENT_SOURCE_DIR}/include")
target_compile_definitions(csweb_client INTERFACE
    _CSWEB_ENABLE=1
    CSWEB_LIB_FILE="${_file}")