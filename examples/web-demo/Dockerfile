FROM oven/bun:1
WORKDIR /app/sdks/ts
COPY sdks/ts/package.json ./package.json
COPY sdks/ts/bun.lock ./bun.lock
COPY sdks/ts/src ./src
RUN bun install --frozen-lockfile

WORKDIR /app/examples/web-demo
COPY examples/web-demo/package.json ./package.json
COPY examples/web-demo/bun.lock ./bun.lock
RUN bun install --frozen-lockfile
COPY examples/web-demo/index.html ./index.html
COPY examples/web-demo/src ./src
EXPOSE 8080
CMD ["bun", "index.html", "--host=0.0.0.0:8080"]
