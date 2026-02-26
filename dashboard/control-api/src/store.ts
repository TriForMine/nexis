export type ProjectRecord = {
  id: string;
  name: string;
  created_at: string;
};

export type ProjectKeyRecord = {
  id: string;
  project_id: string;
  name: string;
  secret: string;
  scopes: string[];
  revoked_at: string | null;
  rotated_from: string | null;
  created_at: string;
};

export type ProjectKeyPublicRecord = {
  id: string;
  project_id: string;
  name: string;
  scopes: string[];
  revoked_at: string | null;
  rotated_from: string | null;
  created_at: string;
};

export interface ControlStore {
  createProject(id: string, name: string): Promise<ProjectRecord>;
  listProjects(): Promise<ProjectRecord[]>;
  projectExists(projectId: string): Promise<boolean>;
  createProjectKey(
    id: string,
    projectId: string,
    name: string,
    secret: string,
    scopes: string[],
    rotatedFrom: string | null,
  ): Promise<ProjectKeyRecord>;
  listProjectKeys(projectId: string): Promise<ProjectKeyPublicRecord[]>;
  keyExists(projectId: string, keyId: string): Promise<boolean>;
  getProjectKey(
    projectId: string,
    keyId: string,
  ): Promise<ProjectKeyRecord | null>;
  revokeProjectKey(
    projectId: string,
    keyId: string,
    revokedAt: string,
  ): Promise<ProjectKeyPublicRecord | null>;
}
