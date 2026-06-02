import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

type Args = {
  config: string;
  outDir: string;
  tsReport: string;
  rustReport: string;
  rustSummary: string;
  summary: string;
  skipRun: boolean;
  reportAll: boolean;
};

type CsvRow = Record<string, string>;

type Difference = {
  signature: string;
  count: number;
};

type ParitySummary = {
  generatedAt: string;
  configPath: string;
  reports: {
    typescript: string;
    rust: string;
    rustSummary: string;
    comparison: string;
  };
  counts: {
    typescriptRows: number;
    rustRows: number;
    typescriptActions: Record<string, number>;
    rustActions: Record<string, number>;
    typescriptReasons: Record<string, number>;
    rustReasons: Record<string, number>;
    typescriptResolutions: Record<string, number>;
    rustResolutions: Record<string, number>;
  };
  matches: {
    rowCount: boolean;
    actionCounts: boolean;
    reasonCounts: boolean;
    resolutionCounts: boolean;
    commonRows: boolean;
  };
  differences: {
    missingInRust: Difference[];
    extraInRust: Difference[];
  };
};

const COMMON_COLUMNS = [
  "pair_id",
  "action",
  "canonical_uid",
  "reason",
  "resolution",
  "conflict_policy",
  "title",
  "google_present",
  "icloud_present",
  "google_title",
  "icloud_title",
  "google_start",
  "icloud_start",
  "google_end",
  "icloud_end",
  "google_status",
  "icloud_status",
  "google_hash",
  "icloud_hash",
  "diff_fields"
] as const;

const COUNT_LIMIT = 50;

async function main(): Promise<void> {
  const args = parseArgs(Bun.argv.slice(2));

  mkdirSync(args.outDir, { recursive: true });
  mkdirSync(dirname(args.summary), { recursive: true });

  if (!args.skipRun) {
    runCommand(
      [
        "bun",
        "src/index.ts",
        "sync",
        "--once",
        "--report",
        args.tsReport,
        ...(args.reportAll ? ["--report-all"] : [])
      ],
      {
        INSYNC_CONFIG: args.config,
        INSYNC_LOG_LEVEL: process.env.INSYNC_LOG_LEVEL ?? "warn"
      }
    );

    runCommand([
      "cargo",
      "run",
      "--manifest-path",
      "rust/Cargo.toml",
      "-p",
      "insync-cli",
      "--",
      "--config",
      args.config,
      "sync",
      "--report",
      args.rustReport,
      "--summary-json",
      args.rustSummary,
      ...(args.reportAll ? ["--report-all"] : [])
    ]);
  }

  assertFile(args.tsReport, "TypeScript report");
  assertFile(args.rustReport, "Rust report");

  const tsRows = parseCsv(await Bun.file(args.tsReport).text());
  const rustRows = parseCsv(await Bun.file(args.rustReport).text());
  const summary = compareReports(args, tsRows, rustRows);

  writeFileSync(args.summary, `${JSON.stringify(summary, null, 2)}\n`);
  printSummary(summary);

  if (!Object.values(summary.matches).every(Boolean)) {
    process.exitCode = 2;
  }
}

function parseArgs(argv: string[]): Args {
  const outDir = readFlag(argv, "--out-dir") ?? ".insync/parity";
  const config = readFlag(argv, "--config") ?? process.env.INSYNC_CONFIG ?? "./insync.local.json";
  const tsReport = readFlag(argv, "--ts-report") ?? `${outDir}/typescript-dry-run.csv`;
  const rustReport = readFlag(argv, "--rust-report") ?? `${outDir}/rust-dry-run.csv`;
  const rustSummary = readFlag(argv, "--rust-summary") ?? `${outDir}/rust-summary.json`;
  const summary = readFlag(argv, "--summary") ?? `${outDir}/comparison.json`;

  return {
    config: resolve(config),
    outDir,
    tsReport,
    rustReport,
    rustSummary,
    summary,
    skipRun: argv.includes("--skip-run"),
    reportAll: argv.includes("--report-all")
  };
}

function readFlag(argv: string[], name: string): string | undefined {
  const index = argv.indexOf(name);
  if (index === -1) {
    return undefined;
  }

  const value = argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }

  return value;
}

function runCommand(command: string[], env: Record<string, string> = {}): void {
  const result = Bun.spawnSync({
    cmd: command,
    stdout: "inherit",
    stderr: "inherit",
    env: {
      ...process.env,
      ...env
    }
  });

  if (result.exitCode !== 0) {
    throw new Error(`command failed (${result.exitCode}): ${command.join(" ")}`);
  }
}

function assertFile(path: string, label: string): void {
  if (!existsSync(path)) {
    throw new Error(`${label} does not exist: ${path}`);
  }
}

function compareReports(args: Args, tsRows: CsvRow[], rustRows: CsvRow[]): ParitySummary {
  const missingInRust = multisetDiff(signatureCounts(tsRows), signatureCounts(rustRows));
  const extraInRust = multisetDiff(signatureCounts(rustRows), signatureCounts(tsRows));
  const typescriptActions = countBy(tsRows, "action");
  const rustActions = countBy(rustRows, "action");
  const typescriptReasons = countBy(tsRows, "reason");
  const rustReasons = countBy(rustRows, "reason");
  const typescriptResolutions = countBy(tsRows, "resolution");
  const rustResolutions = countBy(rustRows, "resolution");

  return {
    generatedAt: new Date().toISOString(),
    configPath: args.config,
    reports: {
      typescript: args.tsReport,
      rust: args.rustReport,
      rustSummary: args.rustSummary,
      comparison: args.summary
    },
    counts: {
      typescriptRows: tsRows.length,
      rustRows: rustRows.length,
      typescriptActions,
      rustActions,
      typescriptReasons,
      rustReasons,
      typescriptResolutions,
      rustResolutions
    },
    matches: {
      rowCount: tsRows.length === rustRows.length,
      actionCounts: mapsEqual(typescriptActions, rustActions),
      reasonCounts: mapsEqual(typescriptReasons, rustReasons),
      resolutionCounts: mapsEqual(typescriptResolutions, rustResolutions),
      commonRows: missingInRust.length === 0 && extraInRust.length === 0
    },
    differences: {
      missingInRust: missingInRust.slice(0, COUNT_LIMIT),
      extraInRust: extraInRust.slice(0, COUNT_LIMIT)
    }
  };
}

function signatureCounts(rows: CsvRow[]): Map<string, number> {
  const counts = new Map<string, number>();

  for (const row of rows) {
    const signature = COMMON_COLUMNS.map((column) => normalizeCell(row[column])).join("\u001f");
    counts.set(signature, (counts.get(signature) ?? 0) + 1);
  }

  return counts;
}

function countBy(rows: CsvRow[], column: string): Record<string, number> {
  const counts: Record<string, number> = {};

  for (const row of rows) {
    const key = normalizeCell(row[column]) || "(empty)";
    counts[key] = (counts[key] ?? 0) + 1;
  }

  return sortRecord(counts);
}

function multisetDiff(left: Map<string, number>, right: Map<string, number>): Difference[] {
  const differences: Difference[] = [];

  for (const [signature, count] of left) {
    const delta = count - (right.get(signature) ?? 0);
    if (delta > 0) {
      differences.push({ signature: formatSignature(signature), count: delta });
    }
  }

  return differences.sort((a, b) => a.signature.localeCompare(b.signature));
}

function formatSignature(signature: string): string {
  const cells = signature.split("\u001f");
  return COMMON_COLUMNS.map((column, index) => `${column}=${cells[index] ?? ""}`).join(" | ");
}

function mapsEqual(left: Record<string, number>, right: Record<string, number>): boolean {
  return JSON.stringify(sortRecord(left)) === JSON.stringify(sortRecord(right));
}

function sortRecord(input: Record<string, number>): Record<string, number> {
  return Object.fromEntries(Object.entries(input).sort(([left], [right]) => left.localeCompare(right)));
}

function normalizeCell(value: string | undefined): string {
  return (value ?? "").replace(/\r\n/g, "\n").trim();
}

function parseCsv(input: string): CsvRow[] {
  const records = parseCsvRecords(input);
  const [headers, ...rows] = records;

  if (!headers || headers.length === 0) {
    return [];
  }

  return rows
    .filter((row) => row.some((cell) => cell.length > 0))
    .map((row) => {
      const output: CsvRow = {};

      headers.forEach((header, index) => {
        output[header] = row[index] ?? "";
      });

      return output;
    });
}

function parseCsvRecords(input: string): string[][] {
  const records: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;

  for (let index = 0; index < input.length; index += 1) {
    const char = input[index] ?? "";
    const next = input[index + 1] ?? "";

    if (quoted) {
      if (char === "\"" && next === "\"") {
        cell += "\"";
        index += 1;
      } else if (char === "\"") {
        quoted = false;
      } else {
        cell += char;
      }
      continue;
    }

    if (char === "\"") {
      quoted = true;
    } else if (char === ",") {
      row.push(cell);
      cell = "";
    } else if (char === "\n") {
      row.push(cell);
      records.push(row);
      row = [];
      cell = "";
    } else if (char !== "\r") {
      cell += char;
    }
  }

  if (cell.length > 0 || row.length > 0) {
    row.push(cell);
    records.push(row);
  }

  return records;
}

function printSummary(summary: ParitySummary): void {
  console.log(`comparison: ${summary.reports.comparison}`);
  console.log(`typescript rows: ${summary.counts.typescriptRows}`);
  console.log(`rust rows: ${summary.counts.rustRows}`);
  console.log(`action counts match: ${summary.matches.actionCounts ? "yes" : "no"}`);
  console.log(`reason counts match: ${summary.matches.reasonCounts ? "yes" : "no"}`);
  console.log(`resolution counts match: ${summary.matches.resolutionCounts ? "yes" : "no"}`);
  console.log(`common rows match: ${summary.matches.commonRows ? "yes" : "no"}`);

  if (summary.differences.missingInRust.length > 0) {
    console.log(`missing in Rust: ${summary.differences.missingInRust.length}`);
  }

  if (summary.differences.extraInRust.length > 0) {
    console.log(`extra in Rust: ${summary.differences.extraInRust.length}`);
  }
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
