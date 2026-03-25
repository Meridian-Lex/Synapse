import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

const ChatSchema = z.object({ channel: z.string(), message: z.string() });

export const chatTool = {
  name: "synapse_chat",
  description: "Send a message and immediately poll for replies. Returns { sent, replies }.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string" },
      message: { type: "string" },
    },
    required: ["channel", "message"],
  },
};

export async function handleChat(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel, message } = ChatSchema.parse(args);
  await relay.send(channel, message);
  const messages = await relay.poll(channel);
  return JSON.stringify({
    sent: true,
    replies: messages.map((m) => m.text ?? m.payload),
  });
}
