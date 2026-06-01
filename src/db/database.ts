import { Database } from "bun:sqlite";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";

export type AppDatabase = Database;

export function openDatabase(path: string): AppDatabase {
  ensureParentDir(path);

  const db = new Database(path, { create: true });
  db.exec("PRAGMA journal_mode = WAL");
  db.exec("PRAGMA foreign_keys = ON");
  db.exec("PRAGMA busy_timeout = 5000");

  return db;
}

function ensureParentDir(path: string): void {
  const directory = dirname(path);
  if (directory === "." || directory === "") {
    return;
  }

  mkdirSync(directory, { recursive: true });
}
