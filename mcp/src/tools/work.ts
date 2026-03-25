import { z } from "zod";
import { validateCredentials } from "../cli.js";
import * as relay from "../relay-client.js";

const SendWorkSchema = z.object({
  channel: z.string(),
  payload: z.record(z.unknown()),
});

export const sendWorkTool = {
  name: "synapse_send_work",
  description:
    "Send a structured Work (MessagePack) frame. payload is any JSON object.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string" },
      payload: { type: "object" },
    },
    required: ["channel", "payload"],
  },
};

export async function handleSendWork(args: unknown): Promise<string> {
  const credErr = validateCredentials();
  if (credErr) throw new Error(credErr);
  const { channel, payload } = SendWorkSchema.parse(args);
  await relay.sendWork(channel, payload);
  return JSON.stringify({ ok: true });
}
