import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

export const ListenPollSchema = z.object({
  channel: z.string().describe("Channel name, e.g. #general"),
  timeout_seconds: z.number().int().min(1).max(120).default(5)
    .describe("How long to wait for messages before returning (default 5s)"),
});

export const WaitForReplySchema = z.object({
  channel: z.string().describe("Channel name, e.g. #general"),
  timeout_seconds: z.number().int().min(1).max(300).default(30)
    .describe("Maximum time to wait for replies (default 30s)"),
  min_messages: z.number().int().min(1).max(1000).default(1)
    .describe("Exit early once this many messages are received (default 1)"),
});

export const listenPollTool = {
  name: "synapse_listen_poll",
  description:
    "Poll a Synapse channel for messages. Returns all messages buffered since the last poll. " +
    "Waits up to timeout_seconds for new messages if none are available. " +
    "Backed by a persistent relay connection — messages are never missed between calls.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      timeout_seconds: {
        type: "number",
        description: "How long to wait for messages if none are buffered (default 5s, max 120s)",
        default: 5,
      },
    },
    required: ["channel"],
  },
};

export const waitForReplyTool = {
  name: "synapse_wait_for_reply",
  description:
    "Wait for a reply on a Synapse channel. Exits as soon as min_messages arrive or timeout_seconds elapses. " +
    "Backed by a persistent relay connection — messages are captured even between calls. " +
    "Returns { timedOut, messages }.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      timeout_seconds: {
        type: "number",
        description: "Maximum wait time in seconds (default 30s, max 300s)",
        default: 30,
      },
      min_messages: {
        type: "number",
        description: "Exit early once this many messages are received (default 1)",
        default: 1,
      },
    },
    required: ["channel"],
  },
};

export async function handleListenPoll(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel, timeout_seconds } = ListenPollSchema.parse(args);
  const result = await relay.wait(channel, 1, timeout_seconds);
  return JSON.stringify(result.messages.map(m => m.text ?? m.payload));
}

export async function handleWaitForReply(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel, timeout_seconds, min_messages } = WaitForReplySchema.parse(args);
  const result = await relay.wait(channel, min_messages, timeout_seconds);
  return JSON.stringify({ timedOut: result.timedOut, messages: result.messages.map(m => m.text ?? m.payload) });
}
