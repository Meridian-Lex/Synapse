import { z } from "zod";
import { runWithTimeout, validateCredentials } from "../cli.js";

export const ListenPollSchema = z.object({
  channel: z.string().describe("Channel name, e.g. #general"),
  timeout_seconds: z.number().int().min(1).max(120).default(5)
    .describe("How long to collect messages before returning (default 5s)"),
});

export const WaitForReplySchema = z.object({
  channel: z.string().describe("Channel name, e.g. #general"),
  timeout_seconds: z.number().int().min(1).max(300).default(30)
    .describe("Maximum time to wait for replies (default 30s)"),
  min_messages: z.number().int().min(1).max(1000).default(1)
    .describe("Exit early once this many messages are received (default 1, max 1000)"),
});

export const listenPollTool = {
  name: "synapse_listen_poll",
  description:
    "Poll a Synapse channel for messages. Listens for timeout_seconds and returns all messages received. " +
    "Empty array means the channel was quiet — not an error.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      timeout_seconds: {
        type: "number",
        description: "How long to collect messages (default 5s, max 120s)",
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
    "Use after synapse_send_message to receive the response in a conversation loop. " +
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
  if (credErr) return JSON.stringify({ error: credErr });

  const { channel, timeout_seconds } = ListenPollSchema.parse(args);
  const result = await runWithTimeout(
    ["listen", "--channel", channel],
    timeout_seconds * 1000
  );

  return JSON.stringify(result.messages);
}

export async function handleWaitForReply(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) return JSON.stringify({ error: credErr });

  const { channel, timeout_seconds, min_messages } = WaitForReplySchema.parse(args);
  const result = await runWithTimeout(
    ["listen", "--channel", channel],
    timeout_seconds * 1000,
    (_line, collected) => collected.length >= min_messages
  );

  return JSON.stringify({ timedOut: result.timedOut, messages: result.messages });
}
