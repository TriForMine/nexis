FROM oven/bun:1
WORKDIR /app
COPY dashboard/control-api/package.json ./
COPY dashboard/control-api/bun.lock ./
RUN bun install --frozen-lockfile
COPY dashboard/control-api/src ./src
COPY dashboard/control-api/drizzle ./drizzle
COPY dashboard/control-api/tsconfig.json ./
EXPOSE 3000
CMD ["bun", "src/index.ts"]
