import { protect, csTable, csColumn, Encrypted } from "@cipherstash/protect";
import * as fs from "fs";
import * as path from "path";
import * as dotenv from "dotenv";

const schema = csTable("ci_secrets", {
  value: csColumn("value"),
});

async function main(): Promise<void> {
  const repoRoot = path.resolve(import.meta.dirname, "..");
  const encryptedPath = path.join(repoRoot, ".github", "secrets.env.encrypted");

  if (!fs.existsSync(encryptedPath)) {
    console.error(`Error: ${encryptedPath} not found`);
    process.exit(1);
  }

  const client = await protect({ schemas: [schema] });
  const encrypted: Encrypted = JSON.parse(
    fs.readFileSync(encryptedPath, "utf-8")
  );

  const result = await client.decrypt(encrypted);
  if (result.failure) {
    console.error(`Failed to decrypt: ${result.failure.message}`);
    process.exit(1);
  }

  const env = dotenv.parse(String(result.data));
  const githubEnvPath = process.env.GITHUB_ENV;
  const isCI = !!githubEnvPath;

  for (const [key, value] of Object.entries(env)) {
    if (isCI) {
      const delimiter = `EOF_${key}_${Date.now()}`;
      fs.appendFileSync(githubEnvPath, `${key}<<${delimiter}\n${value}\n${delimiter}\n`);
    } else {
      console.log(`${key}=${value}`);
    }
  }

  if (isCI) {
    console.error(`Decrypted ${Object.keys(env).length} secrets to $GITHUB_ENV`);
  }
}

main().catch((err) => {
  console.error("Decryption failed:", err);
  process.exit(1);
});
