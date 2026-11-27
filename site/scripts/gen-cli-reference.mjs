// Generates site/content/docs/cli-reference.json from the CLI's clap definitions.
// No dependencies. Run `node site/scripts/gen-cli-reference.mjs` to write the file, or
// `node site/scripts/gen-cli-reference.mjs --check` to fail if it is stale.
//
// A flag's meaning is its own `///` doc comment in crates/canvas-daemon/src/cli.rs, with its
// clap default added. A flag with no doc comment must have a hand-written meaning in FLAG_MEANINGS,
// keyed by command and flag. Nothing here scrapes a sentence from a command's description: that
// made `--fps`, `--width`, `--height` and `--frames` share one sentence and left `--out` on
// `diag dump-frames` described as a global flag (issue #540).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");
const repo = path.resolve(site, "..");
const CLI_RS = path.join(repo, "crates", "canvas-daemon", "src", "cli.rs");
const BIN_RS = path.join(repo, "crates", "canvas-daemon", "src", "bin", "cli.rs");
const CLI_MD = path.join(site, "content", "docs", "cli.md");
const OUT = path.join(site, "content", "docs", "cli-reference.json");

/** The commands that never build a daemon client, read from bin/cli.rs's dispatch. Their errors
 * are all CliError::Other, which exits 1; they cannot reach the exit-2 transport errors. */
const NO_DAEMON = new Set(["effects-check", "diag-dump-frames", "diag-bundle"]);

/** A hand-written meaning for a flag with no doc comment, keyed by command and flag. A flag with
 * neither a doc comment nor an entry here fails generation: there is no fallback sentence. */
const FLAG_MEANINGS = {};

/** The H1 of each command page: the reader's task, not the command line. */
const COMMAND_TITLES = {
  status: "Read daemon status",
  "devices-list": "List devices",
  "lights-list": "List lights",
  "effects-list": "List effects",
  "effects-activate": "Activate an effect",
  "effects-import": "Import an effect",
  "effects-check": "Check an effect",
  "effects-source": "Read effect source",
  "effects-delete": "Delete an effect",
  "effects-preset-list": "List presets",
  "effects-preset-save": "Save a preset",
  "effects-preset-delete": "Delete a preset",
  "scenes-list": "List scenes",
  "scenes-set": "Set the active scene",
  "scenes-create": "Create a scene",
  "scenes-edit": "Edit a scene",
  "scenes-delete": "Delete a scene",
  "scenes-order": "Order scenes",
  "scenes-group-move": "Move a group",
  "output-pause": "Pause the lighting",
  "output-resume": "Resume the lighting",
  "fans-status": "Read fan status",
  "fans-profile": "Switch fan profile",
  "peripherals-list": "List peripherals",
  "peripherals-show": "Show a peripheral",
  "peripherals-access": "Set peripheral access",
  "diag-readback": "Read colours back",
  "diag-dump-frames": "Render effect frames",
  "diag-bundle": "Bundle logs and config",
  "diag-fail-sensor": "Fail a sensor",
  "config-export": "Export the config",
  "config-import": "Import the config",
  update: "Install an update",
};

/** The clap structs and inline enum variants that own each command's flags. */
const FLAG_SOURCES = {
  "effects-activate": ["struct ActivateArgs"],
  "effects-import": ["struct ImportArgs"],
  "effects-check": ["struct CheckArgs"],
  "effects-source": ["struct SourceArgs"],
  "effects-delete": ["struct EffectDeleteArgs"],
  "effects-preset-save": ["struct PresetSaveArgs"],
  "effects-preset-delete": ["struct PresetDeleteArgs"],
  "scenes-create": ["struct CreateArgs"],
  "scenes-edit": ["struct EditArgs"],
  "scenes-delete": ["struct DeleteArgs"],
  "scenes-order": ["struct OrderArgs"],
  "scenes-group-move": ["struct GroupMoveArgs"],
  "peripherals-access": ["variant PeripheralsCommand::Access"],
  "diag-dump-frames": ["struct DumpFramesArgs"],
  "diag-bundle": ["variant DiagCommand::Bundle"],
  "diag-fail-sensor": ["variant DiagCommand::FailSensor"],
  update: ["struct UpdateArgs"],
};

const EXIT_OK = { code: "0", meaning: "The command succeeded." };
const EXIT_LOCAL = { code: "1", meaning: "A local check failed: a file could not be read or parsed, or an effect or property was unknown." };
const EXIT_API = { code: "1", meaning: "The API refused the request, or a local check failed." };
const EXIT_PORT = { code: "2", meaning: "The API port has no listener, its listener is outside the client's trusted programs, or the daemon went past its request limit." };

/** The index of the matching close bracket, counting the open one. */
function matchBracket(src, start, open, close) {
  let depth = 0;
  for (let i = start; i < src.length; i++) {
    if (src[i] === open) depth++;
    else if (src[i] === close) {
      depth--;
      if (depth === 0) return i;
    }
  }
  throw new Error(`unbalanced ${open} in ${close}`);
}

/** The `{ ... }` body after a declaration, or an empty string when it is not found. */
function blockAfter(src, header) {
  const at = src.indexOf(header);
  if (at === -1) return "";
  const brace = src.indexOf("{", at + header.length);
  if (brace === -1) return "";
  return src.slice(brace + 1, matchBracket(src, brace, "{", "}"));
}

/** A reader's sentence from one clap doc comment. Code spans keep their backticks. */
export function flagDoc(lines) {
  const paragraphs = [];
  let current = [];
  for (const line of lines) {
    if (line.trim() === "") {
      if (current.length) paragraphs.push(current.join(" "));
      current = [];
    } else current.push(line.trim());
  }
  if (current.length) paragraphs.push(current.join(" "));
  let text = (paragraphs[0] || "")
    .replace(/\[`?([^`\]]+)`?\]\([^)]*\)/g, "$1")
    .replace(/\[`([^`]+)`\]/g, "`$1`")
    .replace(/\bissues?\s*#\s*\d+\b/gi, "")
    .replace(/#\d+\b/g, "")
    .replace(/\(\s*\)/g, "")
    .replace(/\s+\./g, ".")
    .replace(/\s+/g, " ")
    .trim();
  if (!text) return "";
  if (!text.endsWith(".")) text += ".";
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Reads the fields of one struct or enum variant body: each field's doc lines and attributes. */
function readFields(body) {
  const fields = [];
  let docs = [];
  let attrs = [];
  for (const raw of body.split(/\r?\n/)) {
    const doc = /^\s*\/\/\/\s?(.*)$/.exec(raw);
    if (doc) { docs.push(doc[1]); continue; }
    const attr = /^\s*#\[([^\]]*)\]\s*$/.exec(raw);
    if (attr) { attrs.push(attr[1]); continue; }
    const field = /^\s*(?:pub\s+)?(\w+)\s*:\s*[^\n]+,?\s*$/.exec(raw);
    if (field) {
      fields.push({ name: field[1], docs: [...docs], attr: attrs.join(" ") });
      docs = [];
      attrs = [];
      continue;
    }
    if (raw.trim() !== "") { docs = []; attrs = []; }
  }
  return fields;
}

/** Long flags of a field, with the clap default. Fields without `long` are positional. */
function flagsOf(body, command) {
  const out = [];
  for (const field of readFields(body)) {
    const attribute = field.attr;
    const long = /long\s*=\s*"([^"]+)"/.exec(attribute)?.[1]
      || (/\blong\b/.test(attribute) ? field.name.replace(/_/g, "-") : "");
    if (!long) continue; // a positional argument, not a flag
    const flag = `--${long}`;
    const found = /default_value_t\s*=\s*(\d+)/.exec(attribute);
    let meaning = flagDoc(field.docs) || FLAG_MEANINGS[`${command} ${flag}`];
    if (!meaning) throw new Error(`no meaning for ${command} ${flag}: add its doc comment or FLAG_MEANINGS entry`);
    if (found) meaning = `${meaning} Defaults to ${found[1]}.`;
    out.push({ flag, meaning, ...(found ? { default: found[1] } : {}) });
  }
  return out;
}

/** The command slugs in cli.md, in the order docs-data.mjs publishes them. */
function commandSlugs() {
  const lines = fs.readFileSync(CLI_MD, "utf8").replace(/\r\n/g, "\n").split("\n");
  const slugify = (text) => String(text).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "section";
  const sections = [];
  let section = null;
  for (const raw of lines) {
    const line = raw.trim();
    const h2 = /^##\s+(.+)$/.exec(line);
    if (h2) { section = { title: h2[1].trim(), commands: [] }; sections.push(section); continue; }
    if (!section) continue;
    const syntax = /^`(canvas-rgb\s+[^`]+)`\s*$/.exec(line);
    if (!syntax) continue;
    const words = syntax[1].replace(/^canvas-rgb\s+/, "").split(/\s+/);
    const positional = [];
    for (const word of words) {
      if (!/^[a-z][a-z0-9-]*$/.test(word)) break;
      positional.push(word);
    }
    section.commands.push({ slug: positional.join("-"), groupSlug: slugify(section.title) });
  }
  const seen = new Map();
  for (const s of sections) for (const c of s.commands) seen.set(c.slug, (seen.get(c.slug) ?? 0) + 1);
  return sections.flatMap((s) => s.commands.map((c) => seen.get(c.slug) > 1 ? `${c.groupSlug}-${c.slug}` : c.slug));
}

/**
 * The VALUE column of a flag table: the placeholder the published syntax gives the flag. A switch
 * takes no value and reads as an em dash. The syntax in cli.md is the clap command line, so this is
 * the definition's own value, not a guess from the meaning.
 */
function flagValues() {
  const values = new Map();
  const lines = fs.readFileSync(CLI_MD, "utf8").replace(/\r\n/g, "\n").split("\n");
  const seen = new Map();
  const commands = [];
  let section = null;
  for (const raw of lines) {
    const line = raw.trim();
    const h2 = /^##\s+(.+)$/.exec(line);
    if (h2) { section = h2[1].trim(); continue; }
    const syntax = /^`(canvas-rgb\s+[^`]+)`\s*$/.exec(line);
    if (!syntax || !section) continue;
    const words = syntax[1].replace(/^canvas-rgb\s+/, "").split(/\s+/);
    const positional = [];
    for (const word of words) {
      if (!/^[a-z][a-z0-9-]*$/.test(word)) break;
      positional.push(word);
    }
    const slug = positional.join("-");
    const groupSlug = String(section).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
    seen.set(slug, (seen.get(slug) ?? 0) + 1);
    commands.push({ slug, groupSlug, syntax: syntax[1] });
  }
  for (const command of commands) {
    const slug = seen.get(command.slug) > 1 ? `${command.groupSlug}-${command.slug}` : command.slug;
    for (const m of command.syntax.matchAll(/(--[\w-]+)(?:\s+(\[[^\]]+\]|[^\s\]]+))?/g)) {
      let value = (m[2] || "").replace(/^\[/, "").replace(/\]$/, "").replace(/\.\.\.$/, "").trim();
      if (!value) value = "—";
      value = value.replace(/<([^>]+)>/g, (_, inner) => `<${inner.toLowerCase()}>`);
      values.set(`${slug} ${m[1]}`, value);
    }
  }
  return values;
}

/** The published document. Pure: the same Rust sources in, the same object out. */
export function generate() {
  const cliRs = fs.readFileSync(CLI_RS, "utf8");
  const binRs = fs.readFileSync(BIN_RS, "utf8");
  const values = flagValues();
  const globalFlags = flagsOf(blockAfter(cliRs, "pub struct Cli"), "global").map((flag) => ({
    ...flag,
    value: values.get(`global ${flag.flag}`) ?? (flag.flag === "--api" ? "<url>" : "—"),
  }));
  if (globalFlags.length !== 2 || globalFlags.some((f) => !["--json", "--api"].includes(f.flag))) {
    throw new Error(`the global flags are not exactly --json and --api: ${globalFlags.map((f) => f.flag).join(", ")}`);
  }
  // A command marked as needing no daemon must really have no client in its handler.
  for (const [slug, fn] of [["effects-check", "effects_check"], ["diag-dump-frames", "diag_dump_frames"], ["diag-bundle", "diag_bundle"]]) {
    const start = binRs.indexOf(`fn ${fn}(`);
    if (start === -1) throw new Error(`no handler ${fn} in bin/cli.rs for ${slug}`);
    const body = binRs.slice(start, binRs.indexOf("\n}", start));
    if (/\bclient\s*\(/.test(body)) throw new Error(`${slug} calls the daemon client but is marked no-daemon`);
  }
  const commands = {};
  const slugs = commandSlugs();
  for (const slug of slugs) {
    const flags = [];
    for (const source of FLAG_SOURCES[slug] || []) {
      const kind = source.split(" ")[0];
      const body = kind === "struct"
        ? blockAfter(cliRs, `pub ${source}`)
        : enumVariantBody(cliRs, source.slice("variant ".length));
      if (!body) throw new Error(`no clap source ${source} for ${slug}`);
      flags.push(...flagsOf(body, slug));
    }
    // No two flags of one command may say the same thing unless the code gives them the same flag.
    const byMeaning = new Map();
    for (const flag of flags) {
      const key = flag.meaning.replace(/\s+/g, " ").trim();
      if (byMeaning.has(key)) throw new Error(`${slug}: ${flag.flag} and ${byMeaning.get(key)} share a meaning`);
      byMeaning.set(key, flag.flag);
    }
    const needsDaemon = !NO_DAEMON.has(slug);
    const title = COMMAND_TITLES[slug];
    if (!title) throw new Error(`no published title for command ${slug}`);
    commands[slug] = {
      title,
      flags: flags.map((flag) => ({ ...flag, value: values.get(`${slug} ${flag.flag}`) ?? "—" })),
      needsDaemon,
      exitCodes: needsDaemon ? [EXIT_OK, EXIT_API, EXIT_PORT] : [EXIT_OK, EXIT_LOCAL],
    };
  }
  return { globalFlags, commands };
}

/** The body of `Enum::Variant { ... }`. */
function enumVariantBody(src, name) {
  const [enumName, variant] = name.split("::");
  const enumBody = blockAfter(src, `pub enum ${enumName}`);
  if (!enumBody) return "";
  const at = enumBody.indexOf(`${variant} {`);
  if (at === -1) return "";
  const brace = enumBody.indexOf("{", at);
  return enumBody.slice(brace + 1, matchBracket(enumBody, brace, "{", "}"));
}

export function serialize(reference) {
  return JSON.stringify(reference, null, 2) + "\n";
}

const invokedDirectly = process.argv[1] && import.meta.url === new URL(`file://${path.resolve(process.argv[1])}`).href;
if (invokedDirectly) {
  const built = generate();
  if (process.argv.includes("--check")) {
    const current = fs.existsSync(OUT) ? fs.readFileSync(OUT, "utf8").replace(/\r\n/g, "\n") : "";
    if (current === serialize(built)) {
      const commands = Object.keys(built.commands).length;
      const flags = Object.values(built.commands).reduce((n, c) => n + c.flags.length, 0);
      console.log(`cli-reference: up to date (${commands} commands, ${flags} command flags, ${built.globalFlags.length} global flags)`);
      process.exitCode = 0;
    } else {
      console.error("cli-reference: stale. Regenerate with: node site/scripts/gen-cli-reference.mjs");
      process.exitCode = 1;
    }
  } else {
    fs.mkdirSync(path.dirname(OUT), { recursive: true });
    fs.writeFileSync(OUT, serialize(built));
    const commands = Object.keys(built.commands).length;
    const flags = Object.values(built.commands).reduce((n, c) => n + c.flags.length, 0);
    console.log(`cli-reference: wrote ${commands} commands (${flags} command flags, ${built.globalFlags.length} global flags) to site/content/docs/cli-reference.json`);
  }
}
