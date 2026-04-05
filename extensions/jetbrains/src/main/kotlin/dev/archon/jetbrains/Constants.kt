package dev.archon.jetbrains

object Constants {
    const val PLUGIN_ID = "dev.archon.jetbrains"
    const val DEFAULT_WEBSOCKET_URL = "ws://localhost:8420/ws/ide"
    const val DEFAULT_BINARY_PATH = "archon"
    const val TOOL_WINDOW_ID = "Archon"
}

enum class ConnectionMode { STDIO, WEBSOCKET }

enum class ConnectionState { DISCONNECTED, CONNECTING, CONNECTED, ERROR }
