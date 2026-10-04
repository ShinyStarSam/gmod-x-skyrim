cmake_minimum_required(VERSION 3.25)
project(gmsky_protocol_test CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

add_executable(protocol_test test/protocol_test.cpp)
target_include_directories(protocol_test PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
if(MSVC)
    target_compile_options(protocol_test PRIVATE /W4 /WX)
endif()
