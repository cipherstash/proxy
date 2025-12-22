import { protect, csTable, csColumn, Encrypted } from "@cipherstash/protect";
import * as fs from "fs";
import * as path from "path";
import * as dotenv from "dotenv";

const schema = csTable("ci_secrets", {
  value: csColumn("value"),
});

function isFileMode(data: unknown): data is Encrypted {
  return data !== null && typeof data === "object" && "k" in data && "c" in data;
}

async function main(): Promise<void> {
  const repoRoot = path.resolve(import.meta.dirname, "..");
  const encryptedPath = path.join(repoRoot, ".github", "secrets.env.encrypted");

  if (!fs.existsSync(encryptedPath)) {
    console.error(`Error: ${encryptedPath} not found`);
    process.exit(1);
  }

  const client = await protect({ schemas: [schema] });
  const encrypted: unknown = JSON.parse(
    fs.readFileSync(encryptedPath, "utf-8")
  );

  let env: Record<string, string>;

  if (isFileMode(encrypted)) {
    // File mode: decrypt single blob, parse as .env
    const result = await client.decrypt(encrypted);
    if (result.failure) {
      console.error(`Failed to decrypt: ${result.failure.message}`);
      process.exit(1);
    }
    env = dotenv.parse(String(result.data));
    console.error("Detected file mode encryption");
  } else {
    // Vars mode: decrypt each variable individually
    env = {};
    const encryptedVars = encrypted as Record<string, Encrypted>;
    for (const [key, payload] of Object.entries(encryptedVars)) {
      const result = await client.decrypt(payload);
      if (result.failure) {
        console.error(`Failed to decrypt ${key}: ${result.failure.message}`);
        process.exit(1);
      }
      env[key] = String(result.data);
    }
    console.error(`Detected vars mode encryption (${Object.keys(env).length} variables)`);
  }
  const githubEnvPath = process.env.GITHUB_ENV;
  const isCI = !!githubEnvPath;

  // Bootstrap secrets (passed in as env vars, need to forward to $GITHUB_ENV)
  const bootstrapSecrets = [
    "CS_CLIENT_ID",
    "CS_CLIENT_KEY",
    "CS_CLIENT_ACCESS_KEY",
    "CS_WORKSPACE_CRN",
  ];

  // Combine bootstrap secrets with decrypted secrets
  const allSecrets: Record<string, string> = { ...env };
  for (const key of bootstrapSecrets) {
    const value = process.env[key];
    if (value) {
      allSecrets[key] = value;
    }
  }

  for (const [key, value] of Object.entries(allSecrets)) {
    if (isCI) {
      const delimiter = `EOF_${key}_${Date.now()}`;
      fs.appendFileSync(githubEnvPath, `${key}<<${delimiter}\n${value}\n${delimiter}\n`);
    } else {
      // Only output non-bootstrap secrets locally (bootstrap are already in env)
      if (!bootstrapSecrets.includes(key)) {
        console.log(`${key}=${value}`);
      }
    }
  }

  if (isCI) {
    console.error(`Wrote ${Object.keys(allSecrets).length} secrets to $GITHUB_ENV`);
  }
}

main().catch((err) => {
  console.error("Decryption failed:", err);
  process.exit(1);
});
