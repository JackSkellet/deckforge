cmake_minimum_required(VERSION 3.20)
project(StreamDeckPlugin VERSION 5.5.4.0 LANGUAGES CXX)
# Build the pinned upstream sources against the installed Linux OBS and Qt.
find_package(libobs REQUIRED)
find_package(obs-frontend-api REQUIRED)
find_package(Qt6 REQUIRED COMPONENTS Core Widgets)
find_package(Threads REQUIRED)
set(D_PLATFORM_LINUX 1)
set(D_PLATFORM_OS Linux)
set(D_PLATFORM_INSTR "${CMAKE_SYSTEM_PROCESSOR}")
math(EXPR D_PLATFORM_BITS "${CMAKE_SIZEOF_VOID_P} * 8")
set(D_PLATFORM_BITS_PTR ${D_PLATFORM_BITS})
set(PROJECT_VERSION_TWEAK 0)
set(VERSION_STRING "5.5.4.0-deckforge-linux")
set(PROJECT_AUTHORS "Corsair Memory Inc.")
set(PROJECT_FULL_NAME "Stream Deck bridge for OBS Studio")
set(PROJECT_DESCRIPTION "Stream Deck integration, built for Linux by Deckforge.")
foreach(name config.hpp version.hpp module.cpp)
  configure_file("${UPSTREAM}/templates/${name}.in" "generated/${name}" @ONLY)
endforeach()
file(GLOB handlers "${UPSTREAM}/source/handlers/*.cpp")
# Upstream probes Windows/macOS library names for optional chapter/T-bar APIs.
file(READ "${UPSTREAM}/source/handlers/handler-obs-frontend.cpp" frontend)
string(FIND "${frontend}" "os_dlopen(\"obs-frontend-api\")" probe)
if(probe EQUAL -1)
  message(FATAL_ERROR "Upstream frontend library probe changed; review the Linux adjustment")
endif()
string(REPLACE "os_dlopen(\"obs-frontend-api\")" "os_dlopen(\"libobs-frontend-api.so\")" frontend "${frontend}")
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/generated/handler-obs-frontend.cpp" "${frontend}")
list(REMOVE_ITEM handlers "${UPSTREAM}/source/handlers/handler-obs-frontend.cpp")
list(APPEND handlers "${CMAKE_CURRENT_BINARY_DIR}/generated/handler-obs-frontend.cpp")
add_library(StreamDeckPlugin MODULE
  "${UPSTREAM}/source/module.cpp" "${UPSTREAM}/source/json-rpc.cpp"
  "${UPSTREAM}/source/server.cpp" "${UPSTREAM}/source/details-popup.cpp"
  ${handlers} "${CMAKE_CURRENT_BINARY_DIR}/generated/module.cpp")
set_target_properties(StreamDeckPlugin PROPERTIES PREFIX "" CXX_STANDARD 17
  CXX_STANDARD_REQUIRED ON AUTOMOC ON AUTOUIC ON
  AUTOUIC_SEARCH_PATHS "${UPSTREAM}/ui")
target_compile_definitions(StreamDeckPlugin PRIVATE ASIO_STANDALONE
  _WEBSOCKETPP_CPP11_STL_ OBS_MINIMUM_SUPPORT=280000)
target_include_directories(StreamDeckPlugin PRIVATE
  "${CMAKE_CURRENT_BINARY_DIR}/generated" "${UPSTREAM}/source" "${UPSTREAM}/source/handlers"
  "${UPSTREAM}/third-party/nlohmann-json/single_include"
  "${UPSTREAM}/third-party/websocketpp"
  "${UPSTREAM}/third-party/asio-1-28-0/asio/include")
target_link_libraries(StreamDeckPlugin PRIVATE OBS::libobs OBS::obs-frontend-api
  Qt6::Core Qt6::Widgets Threads::Threads)
