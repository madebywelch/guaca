FROM node:24-bookworm-slim
WORKDIR /app
ENV COREPACK_HOME=/usr/local/share/corepack
RUN corepack enable
COPY package.json pnpm-lock.yaml ./
COPY patches ./patches
RUN pnpm install --frozen-lockfile
COPY worker ./worker
COPY src ./src
COPY migrations ./migrations
COPY public ./public
COPY selfhost.sql wrangler.selfhost.jsonc tsconfig.json tsconfig.worker.json start.sh ./
RUN mkdir -p /data && chown node:node /data /app
USER node
EXPOSE 8788
CMD ["./start.sh"]
