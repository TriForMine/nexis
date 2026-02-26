import { readFileSync } from "node:fs";

function readSummary(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function metric(summary, name, field) {
  return summary?.metrics?.[name]?.[field];
}

function num(value) {
  return typeof value === "number" ? value : NaN;
}

function fmtMs(value) {
  if (!Number.isFinite(value)) {
    return "n/a";
  }
  return `${value.toFixed(2)} ms`;
}

function fmtRate(value) {
  if (!Number.isFinite(value)) {
    return "n/a";
  }
  return `${value.toFixed(2)}/s`;
}

function fmtDelta(base, pr) {
  if (!Number.isFinite(base) || !Number.isFinite(pr)) {
    return "n/a";
  }
  const delta = pr - base;
  const sign = delta > 0 ? "+" : "";
  return `${sign}${delta.toFixed(2)}`;
}

function status(base, pr, inverseBetter = true) {
  if (!Number.isFinite(base) || !Number.isFinite(pr)) {
    return "UNKNOWN";
  }
  if (inverseBetter) {
    return pr <= base ? "BETTER_OR_EQUAL" : "REGRESSION";
  }
  return pr >= base ? "BETTER_OR_EQUAL" : "REGRESSION";
}

function main() {
  const args = process.argv.slice(2);
  const gate = args.includes("--gate");
  const positional = args.filter((arg) => !arg.startsWith("--"));
  const [basePath, prPath] = positional;
  if (!basePath || !prPath) {
    throw new Error(
      "Usage: node compare-k6.mjs [--gate] <base.json> <pr.json>",
    );
  }

  const base = readSummary(basePath);
  const pr = readSummary(prPath);

  const rows = [
    {
      name: "handshake p95",
      base: num(metric(base, "nexis_handshake_latency_ms", "p(95)")),
      pr: num(metric(pr, "nexis_handshake_latency_ms", "p(95)")),
      format: fmtMs,
      inverseBetter: true,
    },
    {
      name: "handshake p99",
      base: num(metric(base, "nexis_handshake_latency_ms", "p(99)")),
      pr: num(metric(pr, "nexis_handshake_latency_ms", "p(99)")),
      format: fmtMs,
      inverseBetter: true,
    },
    {
      name: "join p95",
      base: num(metric(base, "nexis_join_latency_ms", "p(95)")),
      pr: num(metric(pr, "nexis_join_latency_ms", "p(95)")),
      format: fmtMs,
      inverseBetter: true,
    },
    {
      name: "join p99",
      base: num(metric(base, "nexis_join_latency_ms", "p(99)")),
      pr: num(metric(pr, "nexis_join_latency_ms", "p(99)")),
      format: fmtMs,
      inverseBetter: true,
    },
    {
      name: "room.message(inc) RTT p95",
      base: num(metric(base, "nexis_room_message_rtt_ms", "p(95)")),
      pr: num(metric(pr, "nexis_room_message_rtt_ms", "p(95)")),
      format: fmtMs,
      inverseBetter: true,
    },
    {
      name: "room.message(inc) RTT p99",
      base: num(metric(base, "nexis_room_message_rtt_ms", "p(99)")),
      pr: num(metric(pr, "nexis_room_message_rtt_ms", "p(99)")),
      format: fmtMs,
      inverseBetter: true,
    },
    {
      name: "state.patch rate",
      base: num(metric(base, "nexis_state_patch_count", "rate")),
      pr: num(metric(pr, "nexis_state_patch_count", "rate")),
      format: fmtRate,
      inverseBetter: false,
    },
    {
      name: "ws errors",
      base: num(metric(base, "nexis_ws_errors", "count")),
      pr: num(metric(pr, "nexis_ws_errors", "count")),
      format: (v) => (Number.isFinite(v) ? `${v}` : "n/a"),
      inverseBetter: true,
    },
  ];

  let regressions = 0;
  let gateFailures = 0;
  const lines = [];
  lines.push("## k6 Performance Comparison");
  lines.push("");
  lines.push("| Metric | Base | PR | Delta (PR-Base) | Status |");
  lines.push("|---|---:|---:|---:|---|");

  for (const row of rows) {
    const rowStatus = status(row.base, row.pr, row.inverseBetter);
    if (rowStatus === "REGRESSION") {
      regressions += 1;
    }
    lines.push(
      `| ${row.name} | ${row.format(row.base)} | ${row.format(row.pr)} | ${fmtDelta(row.base, row.pr)} | ${rowStatus} |`,
    );
  }

  const gateChecks = [
    {
      name: "room.message(inc) RTT p95",
      base: num(metric(base, "nexis_room_message_rtt_ms", "p(95)")),
      pr: num(metric(pr, "nexis_room_message_rtt_ms", "p(95)")),
      allowedFactor: 1.15,
    },
    {
      name: "room.message(inc) RTT p99",
      base: num(metric(base, "nexis_room_message_rtt_ms", "p(99)")),
      pr: num(metric(pr, "nexis_room_message_rtt_ms", "p(99)")),
      allowedFactor: 1.15,
    },
    {
      name: "ws errors",
      base: num(metric(base, "nexis_ws_errors", "count")),
      pr: num(metric(pr, "nexis_ws_errors", "count")),
      allowedIncrease: 0,
    },
  ];

  lines.push("");
  lines.push("### Gate (PR fail criteria)");
  lines.push("");
  lines.push("| Metric | Rule | Base | PR | Gate |");
  lines.push("|---|---|---:|---:|---|");

  for (const check of gateChecks) {
    if (check.name === "ws errors") {
      const fail =
        Number.isFinite(check.base) &&
        Number.isFinite(check.pr) &&
        check.pr > check.base + check.allowedIncrease;
      if (fail) {
        gateFailures += 1;
      }
      lines.push(
        `| ${check.name} | PR <= Base + ${check.allowedIncrease} | ${check.base} | ${check.pr} | ${fail ? "FAIL" : "PASS"} |`,
      );
      continue;
    }

    const limit =
      Number.isFinite(check.base) && Number.isFinite(check.allowedFactor)
        ? check.base * check.allowedFactor
        : NaN;
    const fail =
      Number.isFinite(check.pr) && Number.isFinite(limit) && check.pr > limit;
    if (fail) {
      gateFailures += 1;
    }
    lines.push(
      `| ${check.name} | PR <= Base * ${check.allowedFactor} | ${fmtMs(check.base)} | ${fmtMs(check.pr)} | ${fail ? "FAIL" : "PASS"} |`,
    );
  }

  lines.push("");
  lines.push(`Regressions detected: ${regressions}`);
  lines.push(`Gate failures: ${gateFailures}`);
  lines.push("");
  lines.push(
    "Config source: `infra/load/k6-ws.js` with CI env `VUS=50 DURATION=20s ROOM_SHARDS=10 INC_INTERVAL_MS=250`.",
  );

  process.stdout.write(lines.join("\n"));
  if (gate && gateFailures > 0) {
    process.exit(1);
  }
}

main();
