import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

const ChannelSchema = z.object({ channel: z.string() });

export const listChannelsTool = {
  name: "synapse_list_channels",
  description: "List all channels on the Synapse broker.",
  inputSchema: {
    type: "object" as const,
    properties: {},
    required: [],
  },
};

export const listUsersTool = {
  name: "synapse_list_users",
  description: "List users currently in a channel.",
  inputSchema: {
    type: "object" as const,
    properties: { channel: { type: "string" } },
    required: ["channel"],
  },
};

export const joinTool = {
  name: "synapse_join",
  description:
    "Subscribe relay to a channel (auto-triggered on send/poll too).",
  inputSchema: {
    type: "object" as const,
    properties: { channel: { type: "string" } },
    required: ["channel"],
  },
};

export const leaveTool = {
  name: "synapse_leave",
  description: "Unsubscribe relay from a channel, discard buffer.",
  inputSchema: {
    type: "object" as const,
    properties: { channel: { type: "string" } },
    required: ["channel"],
  },
};

export async function handleListChannels(_args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  return JSON.stringify({ channels: await relay.listChannels() });
}

export async function handleListUsers(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel } = ChannelSchema.parse(args);
  return JSON.stringify({ users: await relay.listUsers(channel) });
}

export async function handleJoin(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel } = ChannelSchema.parse(args);
  await relay.subscribe(channel);
  return JSON.stringify({ ok: true });
}

export async function handleLeave(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel } = ChannelSchema.parse(args);
  await relay.leave(channel);
  return JSON.stringify({ ok: true });
}
