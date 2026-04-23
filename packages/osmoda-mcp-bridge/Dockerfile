# osmoda-mcp-bridge — minimal stdio-MCP server image.
#
# Used by registries (Glama, Smithery, etc.) that build + run the server in
# a container to verify it responds to MCP introspection (tools/list).
#
# Note: tool *invocation* requires the osModa daemon set running on the host
# (agentd, keyd, watch, routines, mesh, mcpd, teachd, voice — over Unix
# sockets at /run/osmoda/*.sock). Inside this container those sockets do
# not exist, so any tool call will fail. introspection works fine because
# the daemon clients are lazy — they only connect on tool invocation.
#
# For a full-functionality deployment, install osModa on a real NixOS host:
#   curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash

# ---- build stage ----
FROM node:22-alpine AS build
WORKDIR /app

COPY packages/osmoda-mcp-bridge/package.json ./
RUN npm install --omit=dev --no-audit --no-fund

COPY packages/osmoda-mcp-bridge/dist ./dist

# ---- runtime stage ----
FROM node:22-alpine
WORKDIR /app

# Inherit only what we actually need.
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/dist          ./dist
COPY --from=build /app/package.json  ./

# MCP servers communicate over stdio. No port to expose.
# The server prints its tools catalog on tools/list and otherwise waits for
# JSON-RPC over stdin.
ENTRYPOINT ["node", "dist/index.js"]
