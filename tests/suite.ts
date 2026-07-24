import "dotenv/config";
import fs from "fs";
import { spec } from "node:test/reporters";
import { run } from "node:test";
import process from "node:process";

(async () => {
  const files: string[] = [];

  // If any tests files were provided specfically, only run those.
  for (let i = 2; i < process.argv.length; i++) {
    files.push(process.argv[i]!);
  }

  // Add all test files if none were specifically provided
  if (files.length === 0) {
    fs.readdirSync("./endpoints", { encoding: null, recursive: true }).forEach(
      (path: string) => {
        if (path.match(/\.spec\.ts$/)) {
          files.push(`./endpoints/${path}`);
        }
      },
    );
  }

  console.log("Running test files:", files);

  run({
    files: files,
    concurrency: true,
    forceExit: true,
    timeout: 30_000,
  })
    .on("test:fail", () => {
      process.exitCode = 1;
    })
    .compose(new spec())
    .pipe(process.stdout, { end: true });
})();
