/** Returns an error string if required SYNAPSE_* vars are missing, else null. */
export function validateCredentials(): string | null {
  const required = ["SYNAPSE_AGENT", "SYNAPSE_SECRET"];
  const missing = required.filter((k) => !process.env[k]);
  if (missing.length === 0) return null;
  return `Missing required environment variables: ${missing.join(", ")}. Set them before starting the MCP server.`;
}
