import { createControlApiApp } from "./app";
import { initializeBetterAuth } from "./better_auth";
import { createPostgresStore, migrate, seedDemoData } from "./db";

const port = Number(process.env.PORT ?? 3000);
const demoProjectId = process.env.NEXIS_DEMO_PROJECT_ID ?? "demo-project";
const demoProjectName = process.env.NEXIS_DEMO_PROJECT_NAME ?? "demo";
const demoKeyId = process.env.NEXIS_DEMO_KEY_ID ?? "demo-key";
const demoKeyName = process.env.NEXIS_DEMO_KEY_NAME ?? "demo-default";
const demoSecret = process.env.NEXIS_DEMO_PROJECT_SECRET ?? "demo-secret";
const masterSecret =
  process.env.NEXIS_MASTER_SECRET ?? "nexis-dev-master-secret";
const internalToken =
  process.env.NEXIS_INTERNAL_TOKEN ?? "nexis-dev-internal-token";

if (!process.env.NEXIS_MASTER_SECRET) {
  console.warn(
    "[nexis] WARNING: NEXIS_MASTER_SECRET is not set. Using insecure dev default. Set this env var before deploying to production.",
  );
}
if (!process.env.NEXIS_INTERNAL_TOKEN) {
  console.warn(
    "[nexis] WARNING: NEXIS_INTERNAL_TOKEN is not set. Using insecure dev default. Set this env var before deploying to production.",
  );
}
if (!process.env.NEXIS_DEMO_PROJECT_SECRET) {
  console.warn(
    "[nexis] WARNING: NEXIS_DEMO_PROJECT_SECRET is not set. Using insecure dev default. Set this env var before deploying to production.",
  );
}

await initializeBetterAuth();
await migrate();
await seedDemoData(
  demoProjectId,
  demoProjectName,
  demoKeyId,
  demoKeyName,
  demoSecret,
);

const app = createControlApiApp(createPostgresStore(), {
  demoProjectId,
  demoSecret,
  masterSecret,
  internalToken,
});

app.listen(port);
console.log(`control-api listening on :${port}`);
