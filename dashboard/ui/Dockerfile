FROM oven/bun:1
WORKDIR /app
COPY dashboard/ui/package.json ./
COPY dashboard/ui/src ./src
EXPOSE 5173
CMD ["bun", "src/server.ts"]
