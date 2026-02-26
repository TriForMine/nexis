import { sql } from "drizzle-orm";
import type { AnyPgColumn } from "drizzle-orm/pg-core";
import { jsonb, pgTable, text, timestamp } from "drizzle-orm/pg-core";

export const projects = pgTable("projects", {
  id: text("id").primaryKey(),
  name: text("name").notNull(),
  createdAt: timestamp("created_at", {
    withTimezone: true,
    mode: "string",
  })
    .notNull()
    .defaultNow(),
});

export const projectKeys = pgTable("project_keys", {
  id: text("id").primaryKey(),
  projectId: text("project_id")
    .notNull()
    .references(() => projects.id, { onDelete: "cascade" }),
  name: text("name").notNull(),
  secret: text("secret").notNull(),
  scopes: jsonb("scopes")
    .$type<string[]>()
    .notNull()
    .default(sql`'["token:mint"]'::jsonb`),
  revokedAt: timestamp("revoked_at", {
    withTimezone: true,
    mode: "string",
  }),
  rotatedFrom: text("rotated_from").references(
    (): AnyPgColumn => projectKeys.id,
    { onDelete: "set null" },
  ),
  createdAt: timestamp("created_at", {
    withTimezone: true,
    mode: "string",
  })
    .notNull()
    .defaultNow(),
});
