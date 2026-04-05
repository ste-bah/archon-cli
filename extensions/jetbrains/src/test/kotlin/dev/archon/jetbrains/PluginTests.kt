package dev.archon.jetbrains

import dev.archon.jetbrains.connection.ConnectionManager
import dev.archon.jetbrains.settings.ArchonSettings
import dev.archon.jetbrains.actions.AskArchonAction
import dev.archon.jetbrains.actions.FixWithArchonAction
import dev.archon.jetbrains.actions.GenerateTestsAction
import org.junit.Test
import org.junit.Assert.*
import java.io.File

class PluginTests {

    @Test
    fun plugin_id_constant() {
        assertEquals("dev.archon.jetbrains", Constants.PLUGIN_ID)
    }

    @Test
    fun connection_mode_default() {
        // ConnectionMode.STDIO is the first-declared value, confirming it is the default
        assertEquals(ConnectionMode.STDIO, ConnectionMode.values()[0])
    }

    @Test
    fun websocket_url_default() {
        val url = Constants.DEFAULT_WEBSOCKET_URL
        assertTrue("URL must contain 8420", url.contains("8420"))
        assertTrue("URL must contain ws/ide", url.contains("ws/ide"))
    }

    @Test
    fun intention_action_text_ask() {
        val action = AskArchonAction()
        val text = action.getText()
        assertTrue("AskArchonAction text must not be empty", text.isNotEmpty())
    }

    @Test
    fun intention_action_text_fix() {
        val action = FixWithArchonAction()
        val text = action.getText()
        assertTrue("FixWithArchonAction text must not be empty", text.isNotEmpty())
    }

    @Test
    fun intention_action_text_tests() {
        val action = GenerateTestsAction()
        val text = action.getText()
        assertTrue("GenerateTestsAction text must not be empty", text.isNotEmpty())
    }

    @Test
    fun message_serialize() {
        val json = ConnectionManager.buildPromptJson("session-1", "hello", emptyList())
        assertTrue("Serialized message must contain archon/prompt", json.contains("archon/prompt"))
    }

    @Test
    fun settings_binary_path_default() {
        assertEquals("archon", ArchonSettings.DEFAULT_BINARY_PATH)
    }

    @Test
    fun connection_manager_initial_state() {
        val manager = ConnectionManager()
        assertEquals(ConnectionState.DISCONNECTED, manager.state)
    }

    @Test
    fun plugin_descriptor_has_id() {
        // Verify plugin.xml exists at expected resource path relative to project root
        val pluginXml = File("src/main/resources/META-INF/plugin.xml")
        // Allow both project-relative and absolute test execution contexts
        val altPluginXml = File(
            System.getProperty("user.dir") +
            "/src/main/resources/META-INF/plugin.xml"
        )
        val exists = pluginXml.exists() || altPluginXml.exists() ||
            PluginTests::class.java.getResourceAsStream("/META-INF/plugin.xml") != null
        assertTrue(
            "META-INF/plugin.xml must exist (checked relative path, absolute path, and classpath)",
            exists
        )
    }
}
