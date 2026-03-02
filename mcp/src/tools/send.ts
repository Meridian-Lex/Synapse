import { z } from "zod";
import { runOnce, validateCredentials } from "../cli.js";

export const SendMessageSchema = z.object({
  channel: z.string().trim().min(1, "channel must not be empty").describe("Channel name, e.g. #general"),
  message: z.string().trim().min(1, "message must not be empty").describe("Message body to send"),
});

export const sendMessageTool = {
  name: "synapse_send_message",
  description: "Send a message to a Synapse fleet channel.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general", minLength: 1 },
      message: { type: "string", description: "Message body to send", minLength: 1 },
    },
    required: ["channel", "message"],
  },
};

export async function handleSendMessage(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);

  const { channel, message } = SendMessageSchema.parse(args);
  const result = await runOnce(["send", "--channel", channel, message]);

  if (result.code !== 0) {
    throw new Error(`Send failed (exit ${result.code}): ${result.stderr || result.stdout || "unknown error"}`);
  }

  return result.stdout || "Delivered.";
}
