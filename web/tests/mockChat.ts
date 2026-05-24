type ChatRequest = {
  attachments?: Array<{ dataBase64?: string | null; fileName?: string }>;
  message?: string;
};

type ChatResponse = ReturnType<typeof chatSubmitResponse>;

export function chatSubmitResponse(request: ChatRequest) {
  const missingBytes = request.attachments?.some((attachment) => !attachment.dataBase64) ?? false;
  if (missingBytes) {
    return {
      messageId: "webmsg_blocked",
      accepted: false,
      createdAtMs: 1770000000,
      policyReason: "chat submit denied: attachment bytes were not provided",
      storedPath: "~/.archon/web/chat.messages.jsonl",
      reply: "",
      attachments: [],
    };
  }
  const attachments = request.attachments?.map((attachment) => ({
    ...attachment,
    dataBase64: null,
    storedPath: `~/.archon/web/uploads/webmsg_test/${attachment.fileName ?? "upload"}`,
  })) ?? [];
  return {
    messageId: "webmsg_test",
    accepted: true,
    createdAtMs: 1770000000,
    policyReason: "chat message accepted and recorded by the web workbench",
    storedPath: "~/.archon/web/chat.messages.jsonl",
    reply: "Mock Archon reply from live session",
    attachments,
  };
}

export function appendChatMessages(
  messages: Array<Record<string, unknown>>,
  request: ChatRequest,
  response: ChatResponse,
) {
  if (!response.accepted) {
    return;
  }
  messages.push(
    {
      id: `${response.messageId}:user`,
      role: "user",
      title: "You",
      body: request.message,
      attachments: response.attachments,
      createdAtMs: response.createdAtMs,
      policyReason: response.policyReason,
      storedPath: response.storedPath,
    },
    {
      id: `${response.messageId}:assistant`,
      role: "assistant",
      title: "Archon",
      body: response.reply,
      attachments: [],
      createdAtMs: response.createdAtMs,
      policyReason: "restored from web chat ledger",
      storedPath: response.storedPath,
    },
  );
}

export function chatHistory(messages: Array<Record<string, unknown>>) {
  return {
    storedPath: "~/.archon/web/chat.messages.jsonl",
    truncated: false,
    messages,
  };
}
