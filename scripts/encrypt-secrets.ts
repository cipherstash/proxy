import { protect, csTable, csColumn } from "@cipherstash/protect";
import * as fs from "fs";
import * as path from "path";

const schema = csTable("ci_secrets", {
  value: csColumn("value"),
});

async function main(): Promise<void> {
  const repoRoot = path.resolve(import.meta.dirname, "..");
  const plaintextPath = path.join(repoRoot, ".github", "secrets.env.plaintext");
  const encryptedPath = path.join(repoRoot, ".github", "secrets.env.encrypted");

  if (!fs.existsSync(plaintextPath)) {
    console.error(`Error: ${plaintextPath} not found`);
    process.exit(1);
  }

  const fileContent = fs.readFileSync(plaintextPath, "utf-8");
  const client = await protect({ schemas: [schema] });

  const result = await client.encrypt(fileContent, {
    table: schema,
    column: schema.value,
  });

  if (result.failure) {
    console.error(`Failed to encrypt: ${result.failure.message}`);
    process.exit(1);
  }

  fs.writeFileSync(encryptedPath, JSON.stringify(result.data, null, 2) + "\n");
  console.error(`Encrypted secrets file to ${encryptedPath}`);
}

main().catch((err) => {
  console.error("Encryption failed:", err);
  process.exit(1);
});
