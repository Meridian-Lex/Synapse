import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";

import { sendMessageTool, handleSendMessage } from "./tools/send.js";
import { listenPollTool, waitForReplyTool, handleListenPoll, handleWaitForReply } from "./tools/listen.js";
import { checkRelay } from "./relay-client.js";
import { chatTool, handleChat } from "./tools/chat.js";
import { sendWorkTool, handleSendWork } from "./tools/work.js";
import {
  listChannelsTool,
  listUsersTool,
  joinTool,
  leaveTool,
  handleListChannels,
  handleListUsers,
  handleJoin,
  handleLeave,
} from "./tools/channels.js";

const server = new Server(
  { name: "synapse", version: "0.1.0" },
  { capabilities: { tools: {} } }
);

const relayOk = await checkRelay();
if (!relayOk) {
  process.stderr.write(
    "[synapse-mcp] WARNING: synapse-relay not reachable at " +
      (process.env.SYNAPSE_RELAY_URL ?? "http://127.0.0.1:7779") +
      ". Start with: synapse-relay serve\n"
  );
}

const tools = [
  sendMessageTool,
  listenPollTool,
  waitForReplyTool,
  chatTool,
  sendWorkTool,
  listChannelsTool,
  listUsersTool,
  joinTool,
  leaveTool,
];

server.setRequestHandler(ListToolsRequestSchema, async () => ({ tools }));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    let text: string;
    switch (name) {
      case "synapse_send_message":
        text = await handleSendMessage(args);
        break;
      case "synapse_listen_poll":
        text = await handleListenPoll(args);
        break;
      case "synapse_wait_for_reply":
        text = await handleWaitForReply(args);
        break;
      case "synapse_chat":
        text = await handleChat(args);
        break;
      case "synapse_send_work":
        text = await handleSendWork(args);
        break;
      case "synapse_list_channels":
        text = await handleListChannels(args);
        break;
      case "synapse_list_users":
        text = await handleListUsers(args);
        break;
      case "synapse_join":
        text = await handleJoin(args);
        break;
      case "synapse_leave":
        text = await handleLeave(args);
        break;
      default:
        return {
          content: [{ type: "text" as const, text: `Unknown tool: ${name}` }],
          isError: true,
        };
    }
    return { content: [{ type: "text" as const, text }] };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return {
      content: [{ type: "text" as const, text: `Tool error: ${message}` }],
      isError: true,
    };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
