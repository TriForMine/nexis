import { describe, expect, test } from "bun:test";

import { createControlApiApp } from "../src/app";
import type {
  ControlStore,
  ProjectKeyPublicRecord,
  ProjectKeyRecord,
  ProjectRecord,
} from "../src/store";
import { deriveProjectSecret, mintHmacToken } from "../src/token";

function jsonRequest(
  path: string,
  options: {
    method?: string;
    body?: unknown;
    headers?: Record<string, string>;
  } = {},
): Request {
  const headers = new Headers(options.headers ?? {});
  if (options.body !== undefined) {
    headers.set("content-type", "application/json");
  }

  return new Request(`http://control.local${path}`, {
    method: options.method ?? "GET",
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });
}

function createMemoryStore(): ControlStore {
  const projects = new Map<string, ProjectRecord>();
  const keys = new Map<string, ProjectKeyRecord>();
  let tick = 0;

  const nextIso = (): string => {
    const date = new Date(Date.UTC(2026, 1, 25, 17, 0, tick));
    tick += 1;
    return date.toISOString();
  };

  return {
    async createProject(id, name) {
      const record: ProjectRecord = { id, name, created_at: nextIso() };
      projects.set(id, record);
      return record;
    },

    async listProjects() {
      return [...projects.values()].sort((a, b) =>
        a.created_at < b.created_at ? 1 : -1,
      );
    },

    async projectExists(projectId) {
      return projects.has(projectId);
    },

    async createProjectKey(id, projectId, name, secret, scopes, rotatedFrom) {
      const record: ProjectKeyRecord = {
        id,
        project_id: projectId,
        name,
        secret,
        scopes,
        revoked_at: null,
        rotated_from: rotatedFrom,
        created_at: nextIso(),
      };
      keys.set(id, record);
      return record;
    },

    async listProjectKeys(projectId) {
      const filtered: ProjectKeyPublicRecord[] = [...keys.values()]
        .filter((key) => key.project_id === projectId)
        .map(({ id, project_id, name, created_at }) => ({
          id,
          project_id,
          name,
          scopes: keys.get(id)?.scopes ?? ["token:mint"],
          revoked_at: keys.get(id)?.revoked_at ?? null,
          rotated_from: keys.get(id)?.rotated_from ?? null,
          created_at,
        }));
      return filtered.sort((a, b) => (a.created_at < b.created_at ? 1 : -1));
    },

    async keyExists(projectId, keyId) {
      const key = keys.get(keyId);
      return Boolean(key && key.project_id === projectId);
    },

    async getProjectKey(projectId, keyId) {
      const key = keys.get(keyId);
      if (!key || key.project_id !== projectId) {
        return null;
      }
      return key;
    },

    async revokeProjectKey(projectId, keyId, revokedAt) {
      const key = keys.get(keyId);
      if (!key || key.project_id !== projectId) {
        return null;
      }

      if (!key.revoked_at) {
        key.revoked_at = revokedAt;
        keys.set(keyId, key);
      }

      return {
        id: key.id,
        project_id: key.project_id,
        name: key.name,
        scopes: key.scopes,
        revoked_at: key.revoked_at,
        rotated_from: key.rotated_from,
        created_at: key.created_at,
      };
    },
  };
}

function createTestApp() {
  return createControlApiApp(createMemoryStore(), {
    demoProjectId: "demo-project",
    demoSecret: "demo-secret",
    masterSecret: "master-secret",
    internalToken: "internal-test-token",
    now: () => new Date("2026-02-25T17:30:00.000Z"),
    randomUUID: (() => {
      let n = 0;
      return () => `id-${++n}`;
    })(),
    createProjectSecret: (() => {
      let n = 0;
      return () => `secret-${++n}`;
    })(),
  });
}

describe("control api", () => {
  test("creates and lists projects", async () => {
    const app = createTestApp();

    const createResponse = await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );
    expect(createResponse.status).toBe(201);

    const created = (await createResponse.json()) as ProjectRecord;
    expect(created.id).toBe("id-1");
    expect(created.name).toBe("mygame");

    const listResponse = await app.handle(jsonRequest("/projects"));
    expect(listResponse.status).toBe(200);
    const projects = (await listResponse.json()) as ProjectRecord[];
    expect(projects).toHaveLength(1);
    expect(projects[0]?.id).toBe("id-1");
  });

  test("creates key, lists keys, and mints deterministic token", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );

    const createKeyResponse = await app.handle(
      jsonRequest("/projects/id-1/keys", {
        method: "POST",
        body: { name: "default" },
      }),
    );
    expect(createKeyResponse.status).toBe(201);
    const createdKey = (await createKeyResponse.json()) as ProjectKeyRecord;
    expect(createdKey.id).toBe("id-2");
    expect(createdKey.secret).toBe("secret-1");

    const listKeysResponse = await app.handle(
      jsonRequest("/projects/id-1/keys"),
    );
    const listedKeys =
      (await listKeysResponse.json()) as ProjectKeyPublicRecord[];
    expect(listKeysResponse.status).toBe(200);
    expect(listedKeys).toHaveLength(1);
    expect(listedKeys[0]?.id).toBe("id-2");
    expect("secret" in (listedKeys[0] as object)).toBe(false);

    const mintResponse = await app.handle(
      jsonRequest("/tokens", {
        method: "POST",
        body: {
          project_id: "id-1",
          key_id: "id-2",
          ttl_seconds: 120,
        },
      }),
    );
    expect(mintResponse.status).toBe(200);
    const minted = (await mintResponse.json()) as {
      token: string;
      claims: {
        project_id: string;
        issued_at: string;
        expires_at: string;
      };
    };

    expect(minted.claims.project_id).toBe("id-1");
    expect(minted.claims.issued_at).toBe("2026-02-25T17:30:00.000Z");
    expect(minted.claims.expires_at).toBe("2026-02-25T17:32:00.000Z");

    const expected = mintHmacToken(
      minted.claims,
      deriveProjectSecret("master-secret", "id-1"),
    );
    expect(minted.token).toBe(expected);
  });

  test("rejects unknown key when minting token", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );

    const mintResponse = await app.handle(
      jsonRequest("/tokens", {
        method: "POST",
        body: {
          project_id: "id-1",
          key_id: "missing",
        },
      }),
    );

    expect(mintResponse.status).toBe(404);
    const payload = (await mintResponse.json()) as { error: string };
    expect(payload.error).toBe("key not found");
  });

  test("responds to CORS preflight with allow-origin", async () => {
    const app = createTestApp();

    const preflight = await app.handle(
      jsonRequest("/projects", {
        method: "OPTIONS",
        headers: {
          origin: "http://localhost:5173",
          "access-control-request-method": "POST",
        },
      }),
    );

    expect(preflight.status).toBe(204);
    expect(preflight.headers.get("access-control-allow-origin")).toBe(
      "http://localhost:5173",
    );
    expect(preflight.headers.get("access-control-allow-credentials")).toBe(
      "true",
    );
  });

  test("rejects token mint when key is revoked", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );

    const createKeyResponse = await app.handle(
      jsonRequest("/projects/id-1/keys", {
        method: "POST",
        body: { name: "default", scopes: ["token:mint"] },
      }),
    );
    const key = (await createKeyResponse.json()) as { id: string };

    const revokeResponse = await app.handle(
      jsonRequest(`/projects/id-1/keys/${key.id}/revoke`, {
        method: "POST",
      }),
    );
    expect(revokeResponse.status).toBe(200);

    const mintResponse = await app.handle(
      jsonRequest("/tokens", {
        method: "POST",
        body: {
          project_id: "id-1",
          key_id: key.id,
        },
      }),
    );
    expect(mintResponse.status).toBe(403);
    expect((await mintResponse.json()) as { error: string }).toEqual({
      error: "key revoked",
    });
  });

  test("rejects token mint when key lacks token:mint scope", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );

    const createKeyResponse = await app.handle(
      jsonRequest("/projects/id-1/keys", {
        method: "POST",
        body: { name: "read-only", scopes: ["projects:read"] },
      }),
    );
    const key = (await createKeyResponse.json()) as { id: string };

    const mintResponse = await app.handle(
      jsonRequest("/tokens", {
        method: "POST",
        body: {
          project_id: "id-1",
          key_id: key.id,
        },
      }),
    );
    expect(mintResponse.status).toBe(403);
    expect((await mintResponse.json()) as { error: string }).toEqual({
      error: "key missing token:mint scope",
    });
  });

  test("rotates key and only new key can mint", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );

    const createKeyResponse = await app.handle(
      jsonRequest("/projects/id-1/keys", {
        method: "POST",
        body: { name: "default", scopes: ["token:mint"] },
      }),
    );
    const oldKey = (await createKeyResponse.json()) as { id: string };

    const rotateResponse = await app.handle(
      jsonRequest(`/projects/id-1/keys/${oldKey.id}/rotate`, {
        method: "POST",
      }),
    );
    expect(rotateResponse.status).toBe(201);
    const rotated = (await rotateResponse.json()) as { id: string };
    expect(rotated.id).not.toBe(oldKey.id);

    const oldMint = await app.handle(
      jsonRequest("/tokens", {
        method: "POST",
        body: {
          project_id: "id-1",
          key_id: oldKey.id,
        },
      }),
    );
    expect(oldMint.status).toBe(403);

    const newMint = await app.handle(
      jsonRequest("/tokens", {
        method: "POST",
        body: {
          project_id: "id-1",
          key_id: rotated.id,
          audience: "ws://localhost:4000",
        },
      }),
    );
    expect(newMint.status).toBe(200);
  });

  test("reports basic observability metrics", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );

    const createKeyResponse = await app.handle(
      jsonRequest("/projects/id-1/keys", {
        method: "POST",
        body: { name: "default", scopes: ["token:mint"] },
      }),
    );
    const key = (await createKeyResponse.json()) as { id: string };

    await app.handle(
      jsonRequest("/tokens", {
        method: "POST",
        body: {
          project_id: "id-1",
          key_id: key.id,
          audience: "ws://localhost:4000",
        },
      }),
    );

    const metricsResponse = await app.handle(jsonRequest("/metrics"));
    expect(metricsResponse.status).toBe(200);
    const metrics = (await metricsResponse.json()) as {
      counters: Record<string, number>;
    };

    expect(metrics.counters.projects_created).toBe(1);
    expect(metrics.counters.keys_created).toBe(1);
    expect(metrics.counters.tokens_minted).toBe(1);
  });

  test("internal key status requires internal token", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );
    await app.handle(
      jsonRequest("/projects/id-1/keys", {
        method: "POST",
        body: { name: "default", scopes: ["token:mint"] },
      }),
    );

    const denied = await app.handle(
      jsonRequest("/internal/key-status?project_id=id-1&key_id=id-2"),
    );
    expect(denied.status).toBe(401);
  });

  test("internal key status is denied when no token is configured", async () => {
    const store = createMemoryStore();
    const app = createControlApiApp(store, {
      demoProjectId: "demo-project",
      demoSecret: "demo-secret",
      masterSecret: "master-secret",
      internalToken: undefined,
      now: () => new Date("2026-02-25T17:30:00.000Z"),
      randomUUID: () => "id-1",
      createProjectSecret: () => "secret-1",
    });

    const response = await app.handle(
      jsonRequest("/internal/key-status?project_id=id-1&key_id=id-1"),
    );
    expect(response.status).toBe(401);
  });

  test("internal key status returns revoked key details", async () => {
    const app = createTestApp();

    await app.handle(
      jsonRequest("/projects", {
        method: "POST",
        body: { name: "mygame" },
      }),
    );
    await app.handle(
      jsonRequest("/projects/id-1/keys", {
        method: "POST",
        body: { name: "default", scopes: ["token:mint"] },
      }),
    );
    await app.handle(
      jsonRequest("/projects/id-1/keys/id-2/revoke", {
        method: "POST",
      }),
    );

    const response = await app.handle(
      jsonRequest("/internal/key-status?project_id=id-1&key_id=id-2", {
        headers: {
          "x-nexis-internal-token": "internal-test-token",
        },
      }),
    );
    expect(response.status).toBe(200);
    const body = (await response.json()) as {
      exists: boolean;
      revoked: boolean;
      scopes: string[];
    };
    expect(body.exists).toBe(true);
    expect(body.revoked).toBe(true);
    expect(body.scopes).toEqual(["token:mint"]);
  });
});
