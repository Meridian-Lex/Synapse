import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";

import { sendMessageTool, handleSendMessage } from "./tools/send.js";
import { listenPollTool, waitForReplyTool, handleListenPoll, handleWaitForReply } from "./tools/listen.js";
import { listChannelsTool, getChannelHistoryTool, handleListChannels, handleGetChannelHistory } from "./tools/stubs.js";

const server = new Server(
  { name: "synapse", version: "0.1.0" },
  { capabilities: { tools: {} } }
);

const tools = [
  sendMessageTool,
  listenPollTool,
  waitForReplyTool,
  listChannelsTool,
  getChannelHistoryTool,
];

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools }));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  let text: string;
  let isError = false;
  try {
    switch (name) {
      case "synapse_send_message":        text = await handleSendMessage(args); break;
      case "synapse_listen_poll":         text = await handleListenPoll(args); break;
      case "synapse_wait_for_reply":      text = await handleWaitForReply(args); break;
      case "synapse_list_channels":       text = await handleListChannels(args); break;
      case "synapse_get_channel_history": text = await handleGetChannelHistory(args); break;
      default:
        return {
          content: [{ type: "text" as const, text: `Unknown tool: ${name}` }],
          isError: true,
        };
    }
    // Detect known error response patterns from handlers
    if (
      text.startsWith("Missing required") ||
      text.startsWith("Send failed") ||
      text.startsWith("synapse CLI not found")
    ) {
      isError = true;
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return {
      content: [{ type: "text" as const, text: `Tool error: ${message}` }],
      isError: true,
    };
  }

  return { content: [{ type: "text" as const, text }], isError };
});

const transport = new StdioServerTransport();
await server.connect(transport);
