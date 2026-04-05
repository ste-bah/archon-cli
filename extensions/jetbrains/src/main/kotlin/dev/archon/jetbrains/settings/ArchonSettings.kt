package dev.archon.jetbrains.settings

import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage

@State(name = "ArchonSettings", storages = [Storage("archon.xml")])
@Service(Service.Level.APP)
class ArchonSettings : PersistentStateComponent<ArchonSettings.State> {

    data class State(
        var connectionMode: String = "stdio",
        var binaryPath: String = DEFAULT_BINARY_PATH,
        var websocketUrl: String = DEFAULT_WEBSOCKET_URL
    )

    private var state = State()

    override fun getState(): State = state

    override fun loadState(state: State) {
        this.state = state
    }

    companion object {
        const val DEFAULT_BINARY_PATH = "archon"
        const val DEFAULT_WEBSOCKET_URL = "ws://localhost:8420/ws/ide"
    }
}
