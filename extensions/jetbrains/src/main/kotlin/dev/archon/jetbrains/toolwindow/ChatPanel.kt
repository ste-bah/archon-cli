package dev.archon.jetbrains.toolwindow

import com.intellij.openapi.project.Project
import dev.archon.jetbrains.connection.ConnectionManager
import javax.swing.JButton
import javax.swing.JComponent
import javax.swing.JPanel
import javax.swing.JScrollPane
import javax.swing.JTextArea
import javax.swing.SwingUtilities
import java.awt.BorderLayout

/**
 * Main chat UI panel embedded in the Archon tool window.
 *
 * All UI mutations happen on the EDT via [SwingUtilities.invokeLater].
 * All I/O is delegated to [ConnectionManager] which runs on a pooled thread.
 */
class ChatPanel(private val project: Project) {

    private val connectionManager = ConnectionManager()
    private val messagesArea = JTextArea().apply {
        isEditable = false
        lineWrap = true
        wrapStyleWord = true
    }
    private val inputField = JTextArea(3, 40)
    private val sendButton = JButton("Send")

    val component: JComponent = JPanel(BorderLayout()).apply {
        add(JScrollPane(messagesArea), BorderLayout.CENTER)
        add(JPanel(BorderLayout()).apply {
            add(JScrollPane(inputField), BorderLayout.CENTER)
            add(sendButton, BorderLayout.EAST)
        }, BorderLayout.SOUTH)
    }

    init {
        sendButton.addActionListener { sendMessage() }
        connectionManager.onTextDelta = { text ->
            SwingUtilities.invokeLater { messagesArea.append(text) }
        }
        connectionManager.onTurnComplete = { _, _, _ ->
            SwingUtilities.invokeLater { messagesArea.append("\n") }
        }
    }

    private fun sendMessage() {
        val text = inputField.text.trim()
        if (text.isBlank()) return
        inputField.text = ""
        SwingUtilities.invokeLater { messagesArea.append("\nYou: $text\n") }
        connectionManager.sendPrompt("session-1", text)
    }
}
