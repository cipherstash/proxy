import { protect, csTable, csColumn } from "@cipherstash/protect";
import * as fs from "fs";
import * as path from "path";
import * as dotenv from "dotenv";

type Mode = "file" | "vars";

function parseArgs(): Mode {
  const args = process.argv.slice(2);
  if (args.includes("--vars")) return "vars";
  if (args.includes("--file")) return "file";
  return "file"; // default
}

const schema = csTable("ci_secrets", {
  value: csColumn("value"),
});

async function main(): Promise<void> {
  const mode = parseArgs();
  const repoRoot = path.resolve(import.meta.dirname, "..");
  const plaintextPath = path.join(repoRoot, ".github", "secrets.env.plaintext");
  const encryptedPath = path.join(repoRoot, ".github", "secrets.env.encrypted");

  if (!fs.existsSync(plaintextPath)) {
    console.error(`Error: ${plaintextPath} not found`);
    process.exit(1);
  }

  const fileContent = fs.readFileSync(plaintextPath, "utf-8");
  const client = await protect({ schemas: [schema] });

  if (mode === "file") {
    // File mode: encrypt entire file content as single blob
    const result = await client.encrypt(fileContent, {
      table: schema,
      column: schema.value,
    });

    if (result.failure) {
      console.error(`Failed to encrypt: ${result.failure.message}`);
      process.exit(1);
    }

    fs.writeFileSync(encryptedPath, JSON.stringify(result.data, null, 2) + "\n");
    console.error(`Encrypted secrets file to ${encryptedPath} (file mode)`);
  } else {
    // Vars mode: encrypt each variable individually
    const env = dotenv.parse(fileContent);
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
    console.error(`Encrypted ${Object.keys(encrypted).length} secrets to ${encryptedPath} (vars mode)`);
  }
}

main().catch((err) => {
  console.error("Encryption failed:", err);
  process.exit(1);
});
