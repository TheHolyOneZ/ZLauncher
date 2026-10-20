import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const banned = [/backdrop-filter\s*:/, /\bbackdrop-blur\b/, /backdropFilter/];
const bad = [];
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (/\.(css|tsx?|jsx?)$/.test(name)) {
      readFileSync(p, "utf8")
        .split("\n")
        .forEach((line, i) => {
          const code = line.replace(/\/\*.*?\*\/|\/\/.*$/g, "");
          if (banned.some((re) => re.test(code))) bad.push(`${p}:${i + 1}: ${line.trim()}`);
        });
    }
  }
};
walk(fileURLToPath(new URL("../src", import.meta.url)));
if (bad.length) {
  console.error("backdrop blur is not allowed (it exhausts WebKitGTK's memory):\n" + bad.join("\n"));
  process.exit(1);
}
