package dev.archon.sdk

import io.ktor.client.*
import io.ktor.client.engine.cio.*
import io.ktor.client.plugins.websocket.*
import io.ktor.websocket.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.ReceiveChannel
import kotlinx.serialization.*
import kotlinx.serialization.json.*
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger

// ── JSON-RPC 2.0 framing types ────────────────────────────────────────────────

@Serializable
data class JRpcRequest(
    val jsonrpc: String = "2.0",
    val id: Int,
    val method: String,
    val params: JsonElement,
)

@Serializable
data class JRpcResponse(
    val jsonrpc: String = "2.0",
    val id: Int,
    val result: JsonElement? = null,
    val error: JRpcError? = null,
)

@Serializable
data class JRpcNotification(
    val jsonrpc: String = "2.0",
    val method: String,
    val params: JsonElement,
)

@Serializable
data class JRpcError(
    val code: Int,
    val message: String,
    val data: JsonElement? = null,
)

/** Standard JSON-RPC 2.0 error codes. */
object JRpcErrorCode {
    const val PARSE_ERROR = -32700
    const val INVALID_REQUEST = -32600
    const val METHOD_NOT_FOUND = -32601
    const val INVALID_PARAMS = -32602
    const val INTERNAL_ERROR = -32603
}

// ── IDE-specific parameter and result types ───────────────────────────────────

@Serializable
data class IdeCapabilities(
    val inlineCompletion: Boolean = false,
    val toolExecution: Boolean = false,
    val diff: Boolean = false,
    val terminal: Boolean = false,
)

@Serializable
data class IdeClientInfo(
    val name: String,
    val version: String,
)

@Serializable
data class IdeInitializeParams(
    val clientInfo: IdeClientInfo,
    val capabilities: IdeCapabilities,
)

@Serializable
data class IdeInitializeResult(
    val sessionId: String,
    val serverVersion: String,
    val capabilities: IdeCapabilities,
)

@Serializable
data class IdePromptParams(
    val sessionId: String,
    val text: String,
    val contextFiles: List<String>? = null,
)

@Serializable
data class IdeCancelParams(val sessionId: String)

@Serializable
data class IdeCancelResult(val cancelled: Boolean)

@Serializable
data class IdeToolResultParams(
    val sessionId: String,
    val toolUseId: String,
    val result: String,
    val isError: Boolean = false,
)

@Serializable
data class IdeStatusParams(val sessionId: String)

@Serializable
data class IdeStatusResult(
    val model: String,
    val inputTokens: Long,
    val outputTokens: Long,
    val cost: Double,
)

@Serializable
data class IdeConfigParams(
    val key: String? = null,
    val value: JsonElement? = null,
)

// ── Notification payload types ────────────────────────────────────────────────

@Serializable
data class IdeTextDelta(val sessionId: String, val text: String)

@Serializable
data class IdeThinkingDelta(val sessionId: String, val thinking: String)

@Serializable
data class IdeToolCall(
    val sessionId: String,
    val toolUseId: String,
    val name: String,
    val input: JsonElement,
)

@Serializable
data class IdePermissionRequest(
    val sessionId: String,
    val action: String,
    val description: String,
)

@Serializable
data class IdeTurnComplete(
    val sessionId: String,
    val inputTokens: Long,
    val outputTokens: Long,
    val cost: Double,
)

@Serializable
data class IdeErrorNotification(
    val sessionId: String? = null,
    val message: String,
    val code: Int,
)

// ── ArchonClient ──────────────────────────────────────────────────────────────

/**
 * WebSocket-based Archon client for IDE extensions (Kotlin / JVM).
 *
 * Usage:
 * ```kotlin
 * val client = ArchonClient("ws://localhost:7474/ws/ide")
 * client.connect()
 * val result = client.initialize(IdeCapabilities(inlineCompletion = true))
 * client.onTextDelta { sessionId, text -> print(text) }
 * client.sendPrompt(result.sessionId, "Hello!")
 * client.disconnect()
 * ```
 */
class ArchonClient(private val url: String) {

    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = false
    }

    private val httpClient = HttpClient(CIO) {
        install(WebSockets)
    }

    private var wsSession: DefaultClientWebSocketSession? = null
    private val pending = ConcurrentHashMap<Int, CompletableDeferred<JRpcResponse>>()
    private val nextId = AtomicInteger(1)

    // Notification handlers stored as lists of lambdas
    private val textDeltaHandlers = mutableListOf<(String, String) -> Unit>()
    private val thinkingDeltaHandlers = mutableListOf<(String, String) -> Unit>()
    private val toolCallHandlers = mutableListOf<(String, String, String, JsonElement) -> Unit>()
    private val permissionRequestHandlers = mutableListOf<(String, String, String) -> Unit>()
    private val turnCompleteHandlers = mutableListOf<(String, Long, Long, Double) -> Unit>()
    private val errorHandlers = mutableListOf<(String?, String, Int) -> Unit>()

    private var receiveJob: Job? = null

    /**
     * Open the WebSocket connection and start the receive loop in the background.
     * Must be called before any other method.
     */
    suspend fun connect() {
        val session = httpClient.webSocketSession(url)
        wsSession = session
        receiveJob = CoroutineScope(Dispatchers.IO).launch {
            receiveLoop(session.incoming)
        }
    }

    /** Send `archon/initialize` and return the server result. */
    suspend fun initialize(
        clientInfo: IdeClientInfo = IdeClientInfo("archon-sdk-kotlin", "0.1.0"),
        capabilities: IdeCapabilities = IdeCapabilities(),
    ): IdeInitializeResult {
        val params = IdeInitializeParams(clientInfo = clientInfo, capabilities = capabilities)
        val raw = request("archon/initialize", json.encodeToJsonElement(params))
        return json.decodeFromJsonElement(requireNotNull(raw.result) { "null result from archon/initialize" })
    }

    /** Send `archon/prompt` — queues a user prompt in the active session. */
    suspend fun sendPrompt(sessionId: String, text: String, contextFiles: List<String>? = null) {
        val params = IdePromptParams(sessionId = sessionId, text = text, contextFiles = contextFiles)
        request("archon/prompt", json.encodeToJsonElement(params))
    }

    /** Send `archon/cancel` — request cancellation of the current turn. */
    suspend fun cancel(sessionId: String): Boolean {
        val params = IdeCancelParams(sessionId = sessionId)
        val raw = request("archon/cancel", json.encodeToJsonElement(params))
        return json.decodeFromJsonElement<IdeCancelResult>(
            requireNotNull(raw.result) { "null result from archon/cancel" }
        ).cancelled
    }

    /** Send `archon/toolResult` — return a tool execution result to the agent. */
    suspend fun toolResult(sessionId: String, toolUseId: String, result: String, isError: Boolean = false) {
        val params = IdeToolResultParams(
            sessionId = sessionId,
            toolUseId = toolUseId,
            result = result,
            isError = isError,
        )
        request("archon/toolResult", json.encodeToJsonElement(params))
    }

    /** Send `archon/status` — query token usage and cost for a session. */
    suspend fun status(sessionId: String): IdeStatusResult {
        val params = IdeStatusParams(sessionId = sessionId)
        val raw = request("archon/status", json.encodeToJsonElement(params))
        return json.decodeFromJsonElement(requireNotNull(raw.result) { "null result from archon/status" })
    }

    /** Send `archon/config` — read or write a configuration value. */
    suspend fun config(key: String? = null, value: JsonElement? = null): JsonElement? {
        val params = IdeConfigParams(key = key, value = value)
        val raw = request("archon/config", json.encodeToJsonElement(params))
        return raw.result?.let {
            json.decodeFromJsonElement<JsonObject>(it)["value"]
        }
    }

    // ── Notification subscriptions ────────────────────────────────────────────

    /** Register a handler for `archon/textDelta` notifications. */
    fun onTextDelta(handler: (sessionId: String, text: String) -> Unit) {
        textDeltaHandlers += handler
    }

    /** Register a handler for `archon/thinkingDelta` notifications. */
    fun onThinkingDelta(handler: (sessionId: String, thinking: String) -> Unit) {
        thinkingDeltaHandlers += handler
    }

    /** Register a handler for `archon/toolCall` notifications. */
    fun onToolCall(handler: (sessionId: String, toolUseId: String, name: String, input: JsonElement) -> Unit) {
        toolCallHandlers += handler
    }

    /** Register a handler for `archon/permissionRequest` notifications. */
    fun onPermissionRequest(handler: (sessionId: String, action: String, description: String) -> Unit) {
        permissionRequestHandlers += handler
    }

    /** Register a handler for `archon/turnComplete` notifications. */
    fun onTurnComplete(handler: (sessionId: String, inputTokens: Long, outputTokens: Long, cost: Double) -> Unit) {
        turnCompleteHandlers += handler
    }

    /** Register a handler for `archon/error` notifications. */
    fun onError(handler: (sessionId: String?, message: String, code: Int) -> Unit) {
        errorHandlers += handler
    }

    /** Close the WebSocket connection and release resources. */
    suspend fun disconnect() {
        receiveJob?.cancel()
        wsSession?.close()
        httpClient.close()
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    private suspend fun request(method: String, params: JsonElement): JRpcResponse {
        val session = requireNotNull(wsSession) { "not connected — call connect() first" }
        val id = nextId.getAndIncrement()
        val req = JRpcRequest(id = id, method = method, params = params)
        val deferred = CompletableDeferred<JRpcResponse>()
        pending[id] = deferred

        session.send(Frame.Text(json.encodeToString(req)))

        val resp = deferred.await()
        if (resp.error != null) {
            throw IllegalStateException("JSON-RPC error ${resp.error.code}: ${resp.error.message}")
        }
        return resp
    }

    private suspend fun receiveLoop(incoming: ReceiveChannel<Frame>) {
        for (frame in incoming) {
            if (frame is Frame.Text) {
                handleMessage(frame.readText())
            }
        }
    }

    private fun handleMessage(text: String) {
        val element: JsonElement = try {
            json.parseToJsonElement(text)
        } catch (_: Exception) {
            return
        }

        val obj = element as? JsonObject ?: return

        if ("id" in obj) {
            // Response
            val resp: JRpcResponse = try {
                json.decodeFromJsonElement(element)
            } catch (_: Exception) {
                return
            }
            pending.remove(resp.id)?.complete(resp)
        } else if ("method" in obj) {
            // Notification
            val notif: JRpcNotification = try {
                json.decodeFromJsonElement(element)
            } catch (_: Exception) {
                return
            }
            dispatchNotification(notif.method, notif.params)
        }
    }

    private fun dispatchNotification(method: String, params: JsonElement) {
        when (method) {
            "archon/textDelta" -> {
                val p = json.decodeFromJsonElement<IdeTextDelta>(params)
                textDeltaHandlers.forEach { it(p.sessionId, p.text) }
            }
            "archon/thinkingDelta" -> {
                val p = json.decodeFromJsonElement<IdeThinkingDelta>(params)
                thinkingDeltaHandlers.forEach { it(p.sessionId, p.thinking) }
            }
            "archon/toolCall" -> {
                val p = json.decodeFromJsonElement<IdeToolCall>(params)
                toolCallHandlers.forEach { it(p.sessionId, p.toolUseId, p.name, p.input) }
            }
            "archon/permissionRequest" -> {
                val p = json.decodeFromJsonElement<IdePermissionRequest>(params)
                permissionRequestHandlers.forEach { it(p.sessionId, p.action, p.description) }
            }
            "archon/turnComplete" -> {
                val p = json.decodeFromJsonElement<IdeTurnComplete>(params)
                turnCompleteHandlers.forEach { it(p.sessionId, p.inputTokens, p.outputTokens, p.cost) }
            }
            "archon/error" -> {
                val p = json.decodeFromJsonElement<IdeErrorNotification>(params)
                errorHandlers.forEach { it(p.sessionId, p.message, p.code) }
            }
            else -> { /* unknown notification — ignore */ }
        }
    }
}
