package dev.archon.jetbrains.connection

import dev.archon.jetbrains.ConnectionMode
import dev.archon.jetbrains.ConnectionState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

class ConnectionManager {

    var state: ConnectionState = ConnectionState.DISCONNECTED
        private set

    var onTextDelta: ((String) -> Unit)? = null
    var onTurnComplete: ((Long, Long, Double) -> Unit)? = null

    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private var process: Process? = null

    fun connect(mode: ConnectionMode, binaryPath: String, wsUrl: String) {
        state = ConnectionState.CONNECTING
        scope.launch {
            try {
                when (mode) {
                    ConnectionMode.STDIO -> connectStdio(binaryPath)
                    ConnectionMode.WEBSOCKET -> connectWebSocket(wsUrl)
                }
                state = ConnectionState.CONNECTED
            } catch (e: Exception) {
                state = ConnectionState.ERROR
            }
        }
    }

    fun sendPrompt(
        sessionId: String,
        text: String,
        contextFiles: List<String> = emptyList()
    ) {
        scope.launch {
            val msg = buildPromptJson(sessionId, text, contextFiles)
            writeMessage(msg)
        }
    }

    fun disconnect() {
        process?.destroy()
        scope.cancel()
        state = ConnectionState.DISCONNECTED
    }

    // -------------------------------------------------------------------------
    // Private implementation
    // -------------------------------------------------------------------------

    private fun connectStdio(binaryPath: String) {
        val pb = ProcessBuilder(binaryPath, "--ide-mode")
        pb.redirectErrorStream(false)
        process = pb.start()
        scope.launch { readLines() }
    }

    private fun connectWebSocket(wsUrl: String) {
        // Phase 6: wire ktor WebSocket client to $wsUrl
        // WebSocket transport is not yet implemented; connection state set by caller
    }

    private fun readLines() {
        process?.inputStream?.bufferedReader()?.forEachLine { line ->
            handleIncomingLine(line)
        }
    }

    private fun handleIncomingLine(line: String) {
        // Parse JSON-RPC notification and dispatch to registered callbacks.
        // Full parsing implemented in Phase 6 when the protocol is stabilised.
        if (line.contains("\"method\":\"archon/textDelta\"")) {
            val delta = extractStringField(line, "text")
            onTextDelta?.invoke(delta)
        } else if (line.contains("\"method\":\"archon/turnComplete\"")) {
            onTurnComplete?.invoke(0L, 0L, 0.0)
        }
    }

    private fun writeMessage(json: String) {
        process?.outputStream?.write((json + "\n").toByteArray())
        process?.outputStream?.flush()
    }

    /** Minimal field extraction — avoids pulling in a JSON library dependency. */
    private fun extractStringField(json: String, field: String): String {
        val key = "\"$field\":\""
        val start = json.indexOf(key)
        if (start == -1) return ""
        val valueStart = start + key.length
        val end = json.indexOf('"', valueStart)
        return if (end == -1) "" else json.substring(valueStart, end)
    }

    // -------------------------------------------------------------------------
    // Companion — static helpers (also callable from tests without IDE runtime)
    // -------------------------------------------------------------------------

    companion object {
        /**
         * Serialises a prompt request to JSON-RPC 2.0 format.
         * Extracted here so unit tests can call it without an IDE runtime.
         */
        fun buildPromptJson(
            sessionId: String,
            text: String,
            contextFiles: List<String>
        ): String {
            return buildJsonRpcRequest(
                "archon/prompt",
                mapOf(
                    "sessionId" to sessionId,
                    "text" to text,
                    "contextFiles" to contextFiles
                )
            )
        }

        private fun buildJsonRpcRequest(method: String, params: Map<String, Any>): String {
            val paramsJson = params.entries.joinToString(",") { (k, v) ->
                "\"$k\":${valueToJson(v)}"
            }
            return """{"jsonrpc":"2.0","id":1,"method":"$method","params":{$paramsJson}}"""
        }

        private fun valueToJson(v: Any?): String = when (v) {
            null -> "null"
            is String -> "\"${v.replace("\\", "\\\\").replace("\"", "\\\"")}\""
            is Number -> v.toString()
            is Boolean -> v.toString()
            is List<*> -> "[${v.joinToString(",") { valueToJson(it) }}]"
            else -> "\"$v\""
        }
    }
}
