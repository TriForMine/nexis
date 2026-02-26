CREATE TABLE IF NOT EXISTS "projects" (
  "id" text PRIMARY KEY NOT NULL,
  "name" text NOT NULL,
  "created_at" timestamptz DEFAULT now() NOT NULL
);

CREATE TABLE IF NOT EXISTS "project_keys" (
  "id" text PRIMARY KEY NOT NULL,
  "project_id" text NOT NULL,
  "name" text NOT NULL,
  "secret" text NOT NULL,
  "scopes" jsonb DEFAULT '["token:mint"]'::jsonb NOT NULL,
  "revoked_at" timestamptz,
  "rotated_from" text,
  "created_at" timestamptz DEFAULT now() NOT NULL
);

DO $$ BEGIN
 ALTER TABLE "project_keys" ADD CONSTRAINT "project_keys_project_id_projects_id_fk"
 FOREIGN KEY ("project_id") REFERENCES "projects"("id") ON DELETE cascade ON UPDATE no action;
EXCEPTION
 WHEN duplicate_object THEN null;
END $$;

DO $$ BEGIN
 ALTER TABLE "project_keys" ADD CONSTRAINT "project_keys_rotated_from_project_keys_id_fk"
 FOREIGN KEY ("rotated_from") REFERENCES "project_keys"("id") ON DELETE set null ON UPDATE no action;
EXCEPTION
 WHEN duplicate_object THEN null;
END $$;

CREATE INDEX IF NOT EXISTS "project_keys_project_id_idx" ON "project_keys" ("project_id");
CREATE INDEX IF NOT EXISTS "project_keys_project_id_revoked_idx" ON "project_keys" ("project_id", "revoked_at");
