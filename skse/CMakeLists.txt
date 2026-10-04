# gmsky SKSE plugin. Build setup adapted from SkyCraft (MIT): CMake + vcpkg + CommonLibSSE-NG.
cmake_minimum_required(VERSION 3.25)

project(gmsky VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>DLL")

# CommonLibSSE-NG (SE + AE; VR off to keep the build smaller)
set(BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(ENABLE_SKYRIM_VR OFF CACHE BOOL "" FORCE)
set(SKSE_SUPPORT_XBYAK ON CACHE BOOL "" FORCE)
add_subdirectory(extern/CommonLibSSE-NG CommonLibSSE EXCLUDE_FROM_ALL)
if(NOT COMMAND add_commonlibsse_plugin)
	include(extern/CommonLibSSE-NG/cmake/CommonLibSSE.cmake)
endif()

file(GLOB_RECURSE GMSKY_SOURCES CONFIGURE_DEPENDS src/*.cpp src/*.h)

add_commonlibsse_plugin(
	gmsky
	NAME "gmsky"
	AUTHOR "ShinyStarSam"
	USE_ADDRESS_LIBRARY
	SOURCES ${GMSKY_SOURCES}
)

get_filename_component(GMSKY_PROJECT_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." ABSOLUTE)
target_include_directories(gmsky PRIVATE src "${GMSKY_PROJECT_ROOT}/protocol")
target_precompile_headers(gmsky PRIVATE src/PCH.h)
target_compile_definitions(gmsky PRIVATE NOMINMAX WIN32_LEAN_AND_MEAN
	GMSKY_PROJECT_ROOT="${GMSKY_PROJECT_ROOT}")
if(MSVC)
	target_compile_options(gmsky PRIVATE /W4 /permissive- /Zc:preprocessor /utf-8 /MP)
	target_link_options(gmsky PRIVATE /DEBUG:FULL)
endif()

# Copy the built plugin into Skyrim's Data/SKSE/Plugins (the user allowed writes there).
set(GMSKY_SKYRIM_PLUGINS_DIR "B:/SteamLibrary/steamapps/common/Skyrim Special Edition/Data/SKSE/Plugins"
	CACHE PATH "Skyrim Data/SKSE/Plugins folder to copy the plugin into (empty = don't copy)")
if(GMSKY_SKYRIM_PLUGINS_DIR AND EXISTS "${GMSKY_SKYRIM_PLUGINS_DIR}")
	add_custom_command(TARGET gmsky POST_BUILD
		COMMAND ${CMAKE_COMMAND} "-DDLL=$<TARGET_FILE:gmsky>" "-DPDB=$<TARGET_PDB_FILE:gmsky>"
			"-DDEST=${GMSKY_SKYRIM_PLUGINS_DIR}" -P "${CMAKE_CURRENT_SOURCE_DIR}/cmake/deploy.cmake"
		VERBATIM)
endif()
