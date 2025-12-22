import { protect, csTable, csColumn, Encrypted } from "@cipherstash/protect";
import * as fs from "fs";
import * as path from "path";

const schema = csTable("ci_secrets", {
  value: csColumn("value"),
});

type EncryptedSecrets = Record<string, Encrypted>;

async function main(): Promise<void> {
  // Find .env.encrypted relative to repo root (one level up from scripts/)
  const repoRoot = path.resolve(import.meta.dirname, "..");
  const encryptedPath = path.join(repoRoot, ".github", "secrets.env.encrypted");

  if (!fs.existsSync(encryptedPath)) {
    console.error(`Error: ${encryptedPath} not found`);
    process.exit(1);
  }

  const client = await protect({ schemas: [schema] });
  const encrypted: EncryptedSecrets = JSON.parse(
    fs.readFileSync(encryptedPath, "utf-8")
  );

  const githubEnvPath = process.env.GITHUB_ENV;
  const isCI = !!githubEnvPath;

  for (const [key, payload] of Object.entries(encrypted)) {
    const result = await client.decrypt(payload);
    if (result.failure) {
      console.error(`Failed to decrypt ${key}: ${result.failure.message}`);
      process.exit(1);
    }
    const value = String(result.data);

    if (isCI) {
      // GitHub Actions: use heredoc syntax for multiline values
      const delimiter = `EOF_${key}_${Date.now()}`;
      fs.appendFileSync(githubEnvPath, `${key}<<${delimiter}\n${value}\n${delimiter}\n`);
    } else {
      // Local: simple KEY=value output (for testing)
      console.log(`${key}=${value}`);
    }
  }

  if (isCI) {
    console.error(`Decrypted ${Object.keys(encrypted).length} secrets to $GITHUB_ENV`);
  }
}

main().catch((err) => {
  console.error("Decryption failed:", err);
  process.exit(1);
});
