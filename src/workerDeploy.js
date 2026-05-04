import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

export function deployWorker(extraArgs = [], logger = console) {
  const npx = process.platform === "win32" ? "npx.cmd" : "npx";
  const args = [
    "--yes",
    "wrangler",
    "deploy",
    "--config",
    path.join(projectRoot, "wrangler.toml"),
    ...extraArgs,
  ];

  logger.log(`[deploy] running: ${npx} ${args.map(shellQuote).join(" ")}`);

  return new Promise((resolve) => {
    const child = spawn(npx, args, {
      cwd: projectRoot,
      env: process.env,
      stdio: "inherit",
    });

    child.on("error", (error) => {
      logger.error(`[deploy] failed to start wrangler: ${error.message}`);
      resolve(1);
    });

    child.on("close", (code, signal) => {
      if (signal) {
        logger.error(`[deploy] wrangler exited from signal ${signal}`);
        resolve(1);
        return;
      }

      resolve(code ?? 0);
    });
  });
}

function shellQuote(value) {
  const text = String(value);
  return /^[A-Za-z0-9_./:=@-]+$/.test(text) ? text : JSON.stringify(text);
}
