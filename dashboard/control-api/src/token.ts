import { createHmac, randomBytes } from "node:crypto";

export type TokenClaims = {
  project_id: string;
  issued_at: string;
  expires_at: string;
  key_id?: string;
  aud?: string;
};

function toBase64Url(value: Buffer | string): string {
  return Buffer.from(value).toString("base64url");
}

export function createProjectSecret(): string {
  return randomBytes(32).toString("base64url");
}

export function deriveProjectSecret(
  masterSecret: string,
  projectId: string,
): string {
  return createHmac("sha256", masterSecret)
    .update(`nexis.project.${projectId}`)
    .digest("base64url");
}

export function mintHmacToken(claims: TokenClaims, secret: string): string {
  const payload = toBase64Url(JSON.stringify(claims));
  const signature = createHmac("sha256", secret)
    .update(payload)
    .digest("base64url");
  return `${payload}.${signature}`;
}
