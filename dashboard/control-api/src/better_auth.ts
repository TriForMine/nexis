import { betterAuth } from "better-auth";
import { drizzleAdapter } from "better-auth/adapters/drizzle";
import {
  text,
  timestamp,
  pgTable,
  boolean as pgBoolean,
} from "drizzle-orm/pg-core";

import { db } from "./db";

const AUTH_BASE_PATH = "/auth";
const DEFAULT_AUTH_BASE_URL = "http://localhost:3000";
const DEFAULT_ADMIN_EMAIL = "admin@nexis.local";
const DEFAULT_ADMIN_PASSWORD = "ChangeMe!123456";
const DEFAULT_ADMIN_NAME = "Nexis Admin";

const authUser = pgTable("auth_user", {
  id: text("id").primaryKey(),
  name: text("name").notNull(),
  email: text("email").notNull().unique(),
  emailVerified: pgBoolean("email_verified").notNull().default(false),
  image: text("image"),
  createdAt: timestamp("created_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
  updatedAt: timestamp("updated_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
});

const authSession = pgTable("auth_session", {
  id: text("id").primaryKey(),
  expiresAt: timestamp("expires_at", {
    withTimezone: true,
    mode: "date",
  }).notNull(),
  token: text("token").notNull().unique(),
  createdAt: timestamp("created_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
  updatedAt: timestamp("updated_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
  ipAddress: text("ip_address"),
  userAgent: text("user_agent"),
  userId: text("user_id")
    .notNull()
    .references(() => authUser.id, { onDelete: "cascade" }),
});

const authAccount = pgTable("auth_account", {
  id: text("id").primaryKey(),
  accountId: text("account_id").notNull(),
  providerId: text("provider_id").notNull(),
  userId: text("user_id")
    .notNull()
    .references(() => authUser.id, { onDelete: "cascade" }),
  accessToken: text("access_token"),
  refreshToken: text("refresh_token"),
  idToken: text("id_token"),
  accessTokenExpiresAt: timestamp("access_token_expires_at", {
    withTimezone: true,
    mode: "date",
  }),
  refreshTokenExpiresAt: timestamp("refresh_token_expires_at", {
    withTimezone: true,
    mode: "date",
  }),
  scope: text("scope"),
  password: text("password"),
  createdAt: timestamp("created_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
  updatedAt: timestamp("updated_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
});

const authVerification = pgTable("auth_verification", {
  id: text("id").primaryKey(),
  identifier: text("identifier").notNull(),
  value: text("value").notNull(),
  expiresAt: timestamp("expires_at", {
    withTimezone: true,
    mode: "date",
  }).notNull(),
  createdAt: timestamp("created_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
  updatedAt: timestamp("updated_at", { withTimezone: true, mode: "date" })
    .notNull()
    .defaultNow(),
});

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function envBool(name: string, fallback: boolean): boolean {
  const raw = process.env[name];
  if (!raw) {
    return fallback;
  }
  const normalized = raw.trim().toLowerCase();
  return normalized === "1" || normalized === "true" || normalized === "yes";
}

async function readJson(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    return null;
  }
}

type BetterAuthInstance = ReturnType<typeof betterAuth>;

async function ensureDashboardOperatorAccount(
  auth: BetterAuthInstance,
  adminName: string,
  adminEmail: string,
  adminPassword: string,
): Promise<void> {
  const context = await auth.$context;
  const email = adminEmail.trim().toLowerCase();
  const name = adminName.trim() || DEFAULT_ADMIN_NAME;

  if (!email) {
    console.warn(
      "[nexis] WARNING: NEXIS_DASHBOARD_ADMIN_EMAIL is empty. Skipping admin bootstrap.",
    );
    return;
  }

  const minLength = context.password.config.minPasswordLength;
  if (adminPassword.length < minLength) {
    console.warn(
      "[nexis] WARNING: NEXIS_DASHBOARD_ADMIN_PASSWORD does not meet the required password policy. Skipping admin bootstrap.",
    );
    return;
  }

  const existing = await context.internalAdapter.findUserByEmail(email, {
    includeAccounts: true,
  });
  if (existing?.user) {
    return;
  }

  const hashedPassword = await context.password.hash(adminPassword);
  const createdUser = await context.internalAdapter.createUser({
    name,
    email,
    emailVerified: true,
    image: null,
  });
  await context.internalAdapter.linkAccount({
    userId: createdUser.id,
    providerId: "credential",
    accountId: createdUser.id,
    password: hashedPassword,
  });
}

export type BetterAuthRuntime = {
  basePath: string;
  handleRequest: (request: Request) => Promise<Response>;
  isAuthenticated: (request: Request) => Promise<boolean>;
};

type BetterAuthInitConfig = {
  trustedOrigins: string[];
  internalToken: string | null;
};

export async function initializeBetterAuth(
  config: BetterAuthInitConfig,
): Promise<BetterAuthRuntime> {
  const authBaseUrl = process.env.NEXIS_AUTH_BASE_URL ?? DEFAULT_AUTH_BASE_URL;
  const authSecret =
    process.env.BETTER_AUTH_SECRET ??
    process.env.AUTH_SECRET ??
    process.env.NEXIS_MASTER_SECRET ??
    "nexis-dev-auth-secret";
  const allowSignUp = envBool("NEXIS_AUTH_ALLOW_SIGNUP", false);

  if (!process.env.BETTER_AUTH_SECRET && !process.env.AUTH_SECRET) {
    console.warn(
      "[nexis] WARNING: BETTER_AUTH_SECRET (or AUTH_SECRET) is not set. Falling back to NEXIS_MASTER_SECRET/dev value.",
    );
  }

  const auth = betterAuth({
    baseURL: authBaseUrl,
    basePath: AUTH_BASE_PATH,
    secret: authSecret,
    trustedOrigins: config.trustedOrigins,
    database: drizzleAdapter(db, {
      provider: "pg",
      schema: {
        user: authUser,
        session: authSession,
        account: authAccount,
        verification: authVerification,
      },
    }),
    emailAndPassword: {
      enabled: true,
      disableSignUp: !allowSignUp,
      minPasswordLength: 10,
    },
  });

  const adminEmail =
    process.env.NEXIS_DASHBOARD_ADMIN_EMAIL ?? DEFAULT_ADMIN_EMAIL;
  const adminPassword =
    process.env.NEXIS_DASHBOARD_ADMIN_PASSWORD ?? DEFAULT_ADMIN_PASSWORD;
  const adminName =
    process.env.NEXIS_DASHBOARD_ADMIN_NAME ?? DEFAULT_ADMIN_NAME;

  if (!process.env.NEXIS_DASHBOARD_ADMIN_PASSWORD) {
    console.warn(
      "[nexis] WARNING: NEXIS_DASHBOARD_ADMIN_PASSWORD is not set. Using insecure default admin password.",
    );
  }

  try {
    await ensureDashboardOperatorAccount(
      auth,
      adminName,
      adminEmail,
      adminPassword,
    );
  } catch (error) {
    console.warn("[nexis] Better Auth admin bootstrap failed", error);
  }

  async function handleRequest(request: Request): Promise<Response> {
    const pathname = new URL(request.url).pathname;
    const internalAuthorized =
      config.internalToken !== null &&
      request.headers.get("x-nexis-internal-token") === config.internalToken;
    if (
      !allowSignUp &&
      pathname.startsWith(`${AUTH_BASE_PATH}/sign-up`) &&
      !internalAuthorized
    ) {
      return Response.json({ error: "not found" }, { status: 404 });
    }
    return auth.handler(request);
  }

  async function isAuthenticated(request: Request): Promise<boolean> {
    const response = await auth.handler(
      new Request(new URL(`${AUTH_BASE_PATH}/get-session`, request.url), {
        method: "GET",
        headers: request.headers,
      }),
    );
    if (!response.ok) {
      return false;
    }

    const payload = await readJson(response);
    if (!isRecord(payload)) {
      return false;
    }
    return isRecord(payload.session) && isRecord(payload.user);
  }

  return {
    basePath: AUTH_BASE_PATH,
    handleRequest,
    isAuthenticated,
  };
}
