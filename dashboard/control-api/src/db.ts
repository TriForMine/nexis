import { and, desc, eq, sql as dsql } from "drizzle-orm";
import { drizzle } from "drizzle-orm/postgres-js";
import { migrate as drizzleMigrate } from "drizzle-orm/postgres-js/migrator";
import postgres from "postgres";

import type { ControlStore } from "./store";
import { projectKeys, projects } from "./schema";

const databaseUrl =
  process.env.DATABASE_URL ??
  "postgres://postgres:postgres@localhost:5432/nexis";

export const sql = postgres(databaseUrl, {
  max: 10,
  idle_timeout: 20,
});

const db = drizzle(sql, {
  schema: {
    projects,
    projectKeys,
  },
});

function normalizeScopes(value: unknown): string[] {
  if (Array.isArray(value)) {
    const scopes = value.filter(
      (scope): scope is string => typeof scope === "string" && scope.length > 0,
    );
    return scopes.length > 0 ? scopes : ["token:mint"];
  }
  return ["token:mint"];
}

export async function migrate(): Promise<void> {
  await drizzleMigrate(db, { migrationsFolder: "./drizzle" });
}

export async function seedDemoData(
  projectId: string,
  projectName: string,
  keyId: string,
  keyName: string,
  secret: string,
): Promise<void> {
  await db
    .insert(projects)
    .values({
      id: projectId,
      name: projectName,
    })
    .onConflictDoNothing({ target: projects.id });

  await db
    .insert(projectKeys)
    .values({
      id: keyId,
      projectId,
      name: keyName,
      secret,
      scopes: ["token:mint"],
      rotatedFrom: null,
    })
    .onConflictDoNothing({ target: projectKeys.id });
}

export function createPostgresStore(): ControlStore {
  return {
    async createProject(id, name) {
      const [project] = await db
        .insert(projects)
        .values({ id, name })
        .returning();
      return {
        id: project.id,
        name: project.name,
        created_at: project.createdAt,
      };
    },

    async listProjects() {
      const rows = await db
        .select()
        .from(projects)
        .orderBy(desc(projects.createdAt));
      return rows.map((project) => ({
        id: project.id,
        name: project.name,
        created_at: project.createdAt,
      }));
    },

    async projectExists(projectId) {
      const [project] = await db
        .select({ id: projects.id })
        .from(projects)
        .where(eq(projects.id, projectId))
        .limit(1);
      return Boolean(project);
    },

    async createProjectKey(id, projectId, name, secret, scopes, rotatedFrom) {
      const [key] = await db
        .insert(projectKeys)
        .values({
          id,
          projectId,
          name,
          secret,
          scopes,
          rotatedFrom,
        })
        .returning();

      return {
        id: key.id,
        project_id: key.projectId,
        name: key.name,
        secret: key.secret,
        scopes: normalizeScopes(key.scopes),
        revoked_at: key.revokedAt,
        rotated_from: key.rotatedFrom,
        created_at: key.createdAt,
      };
    },

    async listProjectKeys(projectId) {
      const keys = await db
        .select()
        .from(projectKeys)
        .where(eq(projectKeys.projectId, projectId))
        .orderBy(desc(projectKeys.createdAt));

      return keys.map((key) => ({
        id: key.id,
        project_id: key.projectId,
        name: key.name,
        scopes: normalizeScopes(key.scopes),
        revoked_at: key.revokedAt,
        rotated_from: key.rotatedFrom,
        created_at: key.createdAt,
      }));
    },

    async keyExists(projectId, keyId) {
      const [projectKey] = await db
        .select({ id: projectKeys.id })
        .from(projectKeys)
        .where(and(eq(projectKeys.id, keyId), eq(projectKeys.projectId, projectId)))
        .limit(1);
      return Boolean(projectKey);
    },

    async getProjectKey(projectId, keyId) {
      const [projectKey] = await db
        .select()
        .from(projectKeys)
        .where(and(eq(projectKeys.id, keyId), eq(projectKeys.projectId, projectId)))
        .limit(1);
      if (!projectKey) {
        return null;
      }

      return {
        id: projectKey.id,
        project_id: projectKey.projectId,
        name: projectKey.name,
        secret: projectKey.secret,
        scopes: normalizeScopes(projectKey.scopes),
        revoked_at: projectKey.revokedAt,
        rotated_from: projectKey.rotatedFrom,
        created_at: projectKey.createdAt,
      };
    },

    async revokeProjectKey(projectId, keyId, revokedAt) {
      const [projectKey] = await db
        .update(projectKeys)
        .set({
          revokedAt: dsql`COALESCE(${projectKeys.revokedAt}, ${revokedAt}::timestamptz)`,
        })
        .where(and(eq(projectKeys.id, keyId), eq(projectKeys.projectId, projectId)))
        .returning();

      if (!projectKey) {
        return null;
      }

      return {
        id: projectKey.id,
        project_id: projectKey.projectId,
        name: projectKey.name,
        scopes: normalizeScopes(projectKey.scopes),
        revoked_at: projectKey.revokedAt,
        rotated_from: projectKey.rotatedFrom,
        created_at: projectKey.createdAt,
      };
    },
  };
}
