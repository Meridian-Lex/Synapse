import { spawn } from "child_process";
import { createInterface } from "readline";

export interface RunResult {
  stdout: string;
  stderr: string;
  code: number;
}

export interface PollResult {
  messages: string[];
  timedOut: boolean;
}

/** Resolve synapse binary path. SYNAPSE_CLI overrides; falls back to "synapse" in PATH. */
function resolveBin(): string {
  return process.env.SYNAPSE_CLI ?? "synapse";
}

/** Build child process env — passes all SYNAPSE_* vars through. */
function buildEnv(): NodeJS.ProcessEnv {
  return { ...process.env };
}

/** Returns an error string if required SYNAPSE_* vars are missing, else null. */
export function validateCredentials(): string | null {
  const required = ["SYNAPSE_AGENT", "SYNAPSE_SECRET"];
  const missing = required.filter((k) => !process.env[k]);
  if (missing.length === 0) return null;
  return `Missing required environment variables: ${missing.join(", ")}. Set them before starting the MCP server.`;
}

/**
 * Spawn synapse with the given args, wait for exit.
 * Used by send_message.
 */
export async function runOnce(args: string[]): Promise<RunResult> {
  const bin = resolveBin();
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, {
      env: buildEnv(),
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (d: Buffer) => { stdout += d.toString(); });
    child.stderr.on("data", (d: Buffer) => { stderr += d.toString(); });

    child.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code === "ENOENT") {
        reject(new Error(
          `synapse CLI not found. Set SYNAPSE_CLI or add synapse to PATH. ` +
          `See mcp/README.md or run mcp/install.sh to set up the Synapse CLI.`
        ));
      } else {
        reject(err);
      }
    });

    child.on("close", (code: number | null) => {
      resolve({ stdout: stdout.trim(), stderr: stderr.trim(), code: code ?? 1 });
    });
  });
}

/**
 * Spawn synapse with the given args, collect stdout lines for timeoutMs.
 * onLine is called for each line; return true to exit early (e.g. enough messages received).
 * Used by listen_poll and wait_for_reply.
 */
export async function runWithTimeout(
  args: string[],
  timeoutMs: number,
  onLine?: (line: string, collected: string[]) => boolean
): Promise<PollResult> {
  const bin = resolveBin();
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, {
      env: buildEnv(),
      stdio: ["ignore", "pipe", "pipe"],
    });

    const messages: string[] = [];
    let settled = false;

    // Declare timer variable before error handler so the error handler can clear it.
    let timer: ReturnType<typeof setTimeout>;

    child.on("error", (err: NodeJS.ErrnoException) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (err.code === "ENOENT") {
        reject(new Error(
          `synapse CLI not found. Set SYNAPSE_CLI or add synapse to PATH. ` +
          `See mcp/README.md or run mcp/install.sh to set up the Synapse CLI.`
        ));
      } else {
        reject(err);
      }
    });

    const rl = createInterface({ input: child.stdout, crlfDelay: Infinity });

    rl.on("line", (line: string) => {
      if (settled) return;
      // Filter the initial status line from `synapse listen`
      if (line.startsWith("Listening on ")) return;
      if (line === "") return;
      messages.push(line);
      if (onLine && onLine(line, messages)) {
        settled = true;
        clearTimeout(timer);
        rl.close();
        child.kill("SIGTERM");
        resolve({ messages, timedOut: false });
      }
    });

    timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        rl.close();
        child.kill("SIGTERM");
        resolve({ messages, timedOut: true });
      }
    }, timeoutMs);

    child.on("close", () => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        resolve({ messages, timedOut: false });
      }
    });
  });
}
