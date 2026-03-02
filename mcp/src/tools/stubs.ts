const STUB_ACTION =
  "Check TASK-QUEUE.md for a task covering this capability. " +
  "If none exists, create one. If capacity is free, resolve it now.";

function stubResponse(capability: string): string {
  return JSON.stringify({
    status: "not_implemented",
    capability,
    action_required: STUB_ACTION,
  });
}

export const listChannelsTool = {
  name: "synapse_list_channels",
  description:
    "List available Synapse channels. NOT YET IMPLEMENTED — returns a task-queue reminder.",
  inputSchema: {
    type: "object" as const,
    properties: {},
    required: [],
  },
};

export const getChannelHistoryTool = {
  name: "synapse_get_channel_history",
  description:
    "Fetch message history from a Synapse channel. NOT YET IMPLEMENTED — returns a task-queue reminder.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: { type: "string", description: "Channel name, e.g. #general" },
      limit: { type: "number", description: "Max messages to return (default 50)", default: 50 },
    },
    required: ["channel"],
  },
};

// eslint-disable-next-line @typescript-eslint/no-unused-vars
export async function handleListChannels(_args: unknown): Promise<string> {
  return stubResponse("synapse_list_channels");
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
export async function handleGetChannelHistory(_args: unknown): Promise<string> {
  return stubResponse("synapse_get_channel_history");
}
