import net from "node:net";

export async function prepareCdpPort(config, logger = console) {
  if (!config.launch) {
    return config;
  }

  const portFree = await isPortFree(config.cdpHost, config.cdpPort);
  if (portFree) {
    return config;
  }

  if (config.cdpPortExplicit) {
    throw new Error(
      `CDP port ${config.cdpPort} is already in use on ${config.cdpHost}. ` +
        "Stop the process using it or choose another --cdp-port.",
    );
  }

  const nextPort = await findFreePort(config.cdpHost, config.cdpPort + 1, 100);
  logger.warn(`[launcher] CDP port ${config.cdpPort} is already in use; using ${nextPort} instead`);
  config.cdpPort = nextPort;
  return config;
}

export function isPortFree(host, port) {
  return new Promise((resolve, reject) => {
    const server = net.createServer();

    server.once("error", (error) => {
      if (error.code === "EADDRINUSE") {
        resolve(false);
        return;
      }

      reject(new Error(`Unable to probe ${host}:${port}: ${error.message}`));
    });

    server.once("listening", () => {
      server.close(() => resolve(true));
    });

    server.listen(port, host);
  });
}

async function findFreePort(host, startPort, attempts) {
  for (let offset = 0; offset < attempts; offset += 1) {
    const port = startPort + offset;
    if (await isPortFree(host, port)) {
      return port;
    }
  }

  throw new Error(`No free CDP port found from ${startPort} to ${startPort + attempts - 1}`);
}
