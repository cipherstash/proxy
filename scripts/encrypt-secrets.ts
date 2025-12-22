import { protect, csTable, csColumn } from "@cipherstash/protect";
import * as fs from "fs";
import * as path from "path";
import * as dotenv from "dotenv";

const schema = csTable("ci_secrets", {
  value: csColumn("value"),
});

async function main(): Promise<void> {
  const repoRoot = path.resolve(import.meta.dirname, "..");
  const plaintextPath = path.join(repoRoot, ".github", "secrets.env.plaintext");
  const encryptedPath = path.join(repoRoot, ".github", "secrets.env.encrypted");

  if (!fs.existsSync(plaintextPath)) {
    console.error(`Error: ${plaintextPath} not found`);
    console.error("Create this file with your plaintext secrets (KEY=value format)");
    process.exit(1);
  }

  const env = dotenv.parse(fs.readFileSync(plaintextPath));
  const client = await protect({ schemas: [schema] });

  const encrypted: Record<string, unknown> = {};

  for (const [key, value] of Object.entries(env)) {
    const result = await client.encrypt(value, {
      table: schema,
      column: schema.value,
    });
    if (result.failure) {
      console.error(`Failed to encrypt ${key}: ${result.failure.message}`);
      process.exit(1);
    }
    encrypted[key] = result.data;
    console.error(`Encrypted: ${key}`);
  }

  fs.writeFileSync(encryptedPath, JSON.stringify(encrypted, null, 2) + "\n");
  console.error(`\nWrote ${Object.keys(encrypted).length} secrets to ${encryptedPath}`);
}

main().catch((err) => {
  console.error("Encryption failed:", err);
  process.exit(1);
});
