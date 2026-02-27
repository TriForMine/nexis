import { cors } from "@elysiajs/cors";
import { Elysia } from "elysia";

import {
  createProjectSecret as defaultCreateProjectSecret,
  deriveProjectSecret,
  mintHmacToken,
  type TokenClaims,
} from "./token";
import type { ControlStore } from "./store";

type HeadersCarrier = { headers: Record<string, string> };

const defaultAllowedOrigins = [
  "http://localhost:5173",
  "http://127.0.0.1:5173",
  "http://localhost:8080",
];
const TOKEN_MINT_SCOPE = "token:mint";

type MetricsCounters = {
  requests_total: number;
  projects_created: number;
  keys_created: number;
  keys_revoked: number;
  keys_rotated: number;
  tokens_minted: number;
  tokens_denied: number;
};

type MetricsState = {
  started_at: string;
  counters: MetricsCounters;
};

export type ControlApiConfig = {
  allowedOrigins?: string[];
  demoProjectId: string;
  demoSecret: string;
  masterSecret: string;
  internalToken?: string;
  now?: () => Date;
  randomUUID?: () => string;
  createProjectSecret?: () => string;
};

function applyCorsHeaders(
  request: Request,
  set: HeadersCarrier,
  allowedOrigins: Set<string>,
): void {
  const origin = request.headers.get("origin");
  if (!origin || !allowedOrigins.has(origin)) {
    return;
  }

  set.headers["access-control-allow-origin"] = origin;
  set.headers["access-control-allow-credentials"] = "true";
  set.headers.vary = "Origin";
}

function parseScopes(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [TOKEN_MINT_SCOPE];
  }

  const scopes = [...new Set(value)]
    .filter((scope): scope is string => typeof scope === "string")
    .map((scope) => scope.trim())
    .filter((scope) => scope.length > 0);

  if (scopes.length === 0) {
    return [TOKEN_MINT_SCOPE];
  }

  return scopes;
}

function isInternalAuthorized(
  request: Request,
  expectedToken: string | null,
): boolean {
  if (!expectedToken) {
    return false;
  }
  return request.headers.get("x-nexis-internal-token") === expectedToken;
}

export function createControlApiApp(
  store: ControlStore,
  config: ControlApiConfig,
) {
  const now = config.now ?? (() => new Date());
  const randomUUID = config.randomUUID ?? (() => crypto.randomUUID());
  const createProjectSecret =
    config.createProjectSecret ?? defaultCreateProjectSecret;
  const internalToken = config.internalToken?.trim() || null;
  const allowedOrigins = new Set(
    config.allowedOrigins ?? defaultAllowedOrigins,
  );
  const startup = now();
  const metrics: MetricsState = {
    started_at: startup.toISOString(),
    counters: {
      requests_total: 0,
      projects_created: 0,
      keys_created: 0,
      keys_revoked: 0,
      keys_rotated: 0,
      tokens_minted: 0,
      tokens_denied: 0,
    },
  };

  return new Elysia()
    .onRequest(() => {
      metrics.counters.requests_total += 1;
    })
    .use(
      cors({
        aot: false,
        origin: [...allowedOrigins],
        methods: ["GET", "POST", "OPTIONS"],
        allowedHeaders: ["Content-Type", "Authorization"],
        credentials: true,
        preflight: true,
      }),
    )
    .onAfterHandle(({ request, set }) => {
      applyCorsHeaders(request, set as HeadersCarrier, allowedOrigins);
    })
    .get("/health", () => ({ ok: true }))
    .get("/metrics", () => {
      const uptimeSeconds = Math.max(
        0,
        Math.floor((now().getTime() - startup.getTime()) / 1000),
      );
      return {
        started_at: metrics.started_at,
        uptime_seconds: uptimeSeconds,
        counters: metrics.counters,
      };
    })
    .get("/internal/key-status", async ({ request, query, set }) => {
      if (!isInternalAuthorized(request, internalToken)) {
        set.status = 401;
        return { error: "unauthorized" };
      }

      const projectId = String(
        (query as Record<string, unknown>).project_id ?? "",
      ).trim();
      const keyId = String(
        (query as Record<string, unknown>).key_id ?? "",
      ).trim();
      if (!projectId || !keyId) {
        set.status = 400;
        return { error: "project_id and key_id are required" };
      }

      const key = await store.getProjectKey(projectId, keyId);
      if (!key) {
        return {
          project_id: projectId,
          key_id: keyId,
          exists: false,
          revoked: false,
          scopes: [] as string[],
        };
      }

      return {
        project_id: projectId,
        key_id: keyId,
        exists: true,
        revoked: Boolean(key.revoked_at),
        scopes: key.scopes,
      };
    })
    .post("/projects", async ({ body, set }) => {
      const name =
        typeof body === "object" && body && "name" in body
          ? String((body as { name: unknown }).name)
          : "";
      if (!name.trim()) {
        set.status = 400;
        return { error: "name is required" };
      }

      const project = await store.createProject(randomUUID(), name.trim());
      metrics.counters.projects_created += 1;
      set.status = 201;
      return project;
    })
    .get("/projects", async () => {
      return store.listProjects();
    })
    .post("/projects/:id/keys", async ({ params, body, set }) => {
      const projectId = params.id;
      const name =
        typeof body === "object" && body && "name" in body
          ? String((body as { name: unknown }).name)
          : "default";
      const scopes =
        typeof body === "object" && body && "scopes" in body
          ? parseScopes((body as { scopes: unknown }).scopes)
          : [TOKEN_MINT_SCOPE];

      const exists = await store.projectExists(projectId);
      if (!exists) {
        set.status = 404;
        return { error: "project not found" };
      }

      const key = await store.createProjectKey(
        randomUUID(),
        projectId,
        name.trim() || "default",
        createProjectSecret(),
        scopes,
        null,
      );

      metrics.counters.keys_created += 1;
      set.status = 201;
      return key;
    })
    .post("/projects/:id/keys/:keyId/revoke", async ({ params, set }) => {
      const existing = await store.getProjectKey(params.id, params.keyId);
      if (!existing) {
        set.status = 404;
        return { error: "key not found" };
      }

      const revoked = await store.revokeProjectKey(
        params.id,
        params.keyId,
        now().toISOString(),
      );
      if (!revoked) {
        set.status = 404;
        return { error: "key not found" };
      }

      if (!existing.revoked_at) {
        metrics.counters.keys_revoked += 1;
      }
      return revoked;
    })
    .post("/projects/:id/keys/:keyId/rotate", async ({ params, set }) => {
      const existing = await store.getProjectKey(params.id, params.keyId);
      if (!existing) {
        set.status = 404;
        return { error: "key not found" };
      }
      if (existing.revoked_at) {
        set.status = 409;
        return { error: "key already revoked" };
      }

      const revokedAt = now().toISOString();
      await store.revokeProjectKey(params.id, params.keyId, revokedAt);
      const rotated = await store.createProjectKey(
        randomUUID(),
        params.id,
        `${existing.name}-rotated`,
        createProjectSecret(),
        existing.scopes,
        existing.id,
      );

      metrics.counters.keys_rotated += 1;
      metrics.counters.keys_revoked += 1;
      metrics.counters.keys_created += 1;
      set.status = 201;
      return rotated;
    })
    .get("/projects/:id/keys", async ({ params }) => {
      return store.listProjectKeys(params.id);
    })
    .post("/tokens", async ({ body, set }) => {
      const payload = (typeof body === "object" && body ? body : {}) as {
        project_id?: unknown;
        key_id?: unknown;
        ttl_seconds?: unknown;
        audience?: unknown;
      };

      const projectId = String(payload.project_id ?? "").trim();
      const keyId = String(payload.key_id ?? "").trim();
      const audience = String(payload.audience ?? "").trim();
      const ttlSeconds =
        typeof payload.ttl_seconds === "number" &&
        Number.isFinite(payload.ttl_seconds)
          ? Math.max(30, Math.floor(payload.ttl_seconds))
          : 3600;

      if (!projectId || !keyId) {
        metrics.counters.tokens_denied += 1;
        set.status = 400;
        return { error: "project_id and key_id are required" };
      }

      const key = await store.getProjectKey(projectId, keyId);
      if (!key) {
        metrics.counters.tokens_denied += 1;
        set.status = 404;
        return { error: "key not found" };
      }
      if (key.revoked_at) {
        metrics.counters.tokens_denied += 1;
        set.status = 403;
        return { error: "key revoked" };
      }
      if (!key.scopes.includes(TOKEN_MINT_SCOPE)) {
        metrics.counters.tokens_denied += 1;
        set.status = 403;
        return { error: "key missing token:mint scope" };
      }

      const issuedAt = now();
      const effectiveTtl = Math.min(ttlSeconds, 24 * 60 * 60);
      const expiresAt = new Date(issuedAt.getTime() + effectiveTtl * 1000);
      const claims: TokenClaims = {
        project_id: projectId,
        issued_at: issuedAt.toISOString(),
        expires_at: expiresAt.toISOString(),
        key_id: key.id,
        aud: audience || undefined,
      };

      const projectSecret =
        projectId === config.demoProjectId
          ? config.demoSecret
          : deriveProjectSecret(config.masterSecret, projectId);
      const token = mintHmacToken(claims, projectSecret);
      metrics.counters.tokens_minted += 1;
      return { token, claims };
    });
}
