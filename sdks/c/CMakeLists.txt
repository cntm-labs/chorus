cmake_minimum_required(VERSION 3.16)
project(chorus-sdk C)

set(CMAKE_C_STANDARD 11)

find_package(CURL REQUIRED)

# Chorus SDK library
add_library(chorus
    src/chorus.c
    vendor/cJSON.c
)
target_include_directories(chorus PUBLIC include vendor)
target_link_libraries(chorus PRIVATE CURL::libcurl)

# Tests
enable_testing()
add_executable(test_chorus tests/test_chorus.c)
target_link_libraries(test_chorus PRIVATE chorus)
add_test(NAME test_chorus COMMAND test_chorus)
