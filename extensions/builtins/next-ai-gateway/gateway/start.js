#!/usr/bin/env node
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const codexlHome = process.env.CODEXL_HOME || join(homedir(), ".codexl");
const gatewayHome = resolve(
  process.env.CODEXL_NEXT_AI_GATEWAY_HOME || join(codexlHome, "next-ai-gateway"),
);
const defaultConfigPath = join(gatewayHome, "gateway.config.json");
const configPath = resolve(
  process.env.CODEXL_NEXT_AI_GATEWAY_CONFIG_PATH ||
    process.env.GATEWAY_CONFIG_PATH ||
    defaultConfigPath,
);

process.env.GATEWAY_CONFIG_PATH = configPath;
process.env.AGENT_STORAGE_DIR ||= join(gatewayHome, "agent-data");
process.env.RAW_TRACE_SPOOL_DIR ||= join(gatewayHome, "raw-trace-spool");

mkdirSync(gatewayHome, { recursive: true });
mkdirSync(dirname(configPath), { recursive: true });

if (!existsSync(configPath)) {
  copyFileSync(join(__dirname, "gateway.config.default.json"), configPath);
}

await import("./index.cjs");
