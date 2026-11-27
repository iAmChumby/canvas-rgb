// Guards for the generated developer docs reference (issue #540). The JSON is written by
// site/scripts/gen-api-reference.mjs from the spine's routes and the daemon's router. These
// tests check that every route in the spine is represented, that each route and method has a
// hand-written description and a working body example, that the answer shapes come from the
// generated TypeScript, and that the committed file has not drifted.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { generate, serialize, restatesDescription, restatesTitle, matchBracket, meaningKeys, rustStructs } from "../scripts/gen-api-reference.mjs";
import { generate as generateCli, serialize as serializeCli } from "../scripts/gen-cli-reference.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const site = path.resolve(here, "..");
const repo = path.resolve(site, "..");
const jsonPath = process.env.CANVAS_RGB_API_REFERENCE
  ? path.resolve(process.env.CANVAS_RGB_API_REFERENCE)
  : path.join(site, "content", "docs", "api-reference.json");
const committed = fs.readFileSync(jsonPath, "utf8").replace(/\r\n/g, "\n");
const doc = JSON.parse(committed);
const fresh = generate();

/** Route constants in the spine, excluding the /api/v1 prefix itself. */
function spineRoutes() {
  const src = fs.readFileSync(path.join(repo, "crates", "canvas-proto", "src", "api.rs"), "utf8");
  const block = /pub mod routes\s*\{([\s\S]*?)\n\}/.exec(src);
  assert.ok(block, "the routes module is missing from the spine");
  return [...block[1].matchAll(/pub const (\w+): &str = "([^"]+)";/g)]
    .map((m) => ({ name: m[1], path: m[2] }))
    .filter((r) => r.name !== "PREFIX");
}

const at = (routePath) => {
  const entry = doc.entries.find((e) => e.path === routePath);
  assert.ok(entry, `no reference entry for ${routePath}`);
  return entry;
};

test("every route in the spine has an entry in the reference", () => {
  const routes = spineRoutes();
  const paths = new Set(doc.entries.map((e) => e.path));
  for (const route of routes) {
    assert.ok(paths.has(route.path), `spine route ${route.name} (${route.path}) is missing from api-reference.json`);
  }
  assert.equal(paths.size, doc.entries.length, "the reference has duplicate paths");
  assert.ok(!paths.has("/api/v1"), "PREFIX is not a route and must not be an entry");
  assert.ok(paths.has("/api/v1/diag/fail-sensor"), "the inline fail-sensor route is missing");
  assert.equal(doc.entries.length, routes.length + 1, "expected 65 spine paths plus one inline route");
});

test("every route has a unique human title that restates neither path nor method", () => {
  const titles = new Set();
  for (const entry of doc.entries) {
    const title = entry.title;
    assert.ok(title, `${entry.path} has no title`);
    assert.match(title, /^[A-Z][a-z]+(?: [a-z]+){0,3}$/, `${entry.path} has a badly cased title: ${title}`);
    assert.equal(restatesTitle(title, entry.path), false, `${entry.path} has a title that restates its path or method: ${title}`);
    assert.ok(!titles.has(title), `${entry.path} reuses the title ${title}`);
    titles.add(title);
  }
  assert.equal(titles.size, doc.entries.length, "route titles are not unique");
});

test("published entries carry no build-only route names or counts", () => {
  for (const key of ["spineRouteCount", "inlineCount", "route"]) {
    assert.ok(!(key in doc), `the published reference leaks the build-only ${key}`);
  }
  for (const entry of doc.entries) {
    assert.ok(!("route" in entry), `${entry.path} leaks a route constant name`);
    assert.ok(!("inline" in entry), `${entry.path} leaks an inline id`);
    for (const value of Object.values(entry.descriptions)) {
      assert.doesNotMatch(value, /\binline\b/, `${entry.path} names a build-only id`);
    }
  }
});

test("each entry names its area and every method has a plain description, types and params", () => {
  const methods = ["GET", "POST", "PUT", "PATCH", "DELETE"];
  for (const entry of doc.entries) {
    assert.ok(entry.path.startsWith("/api/v1/"), `${entry.path} is not under /api/v1`);
    assert.ok(entry.area, `${entry.path} has no area`);
    assert.ok(entry.methods.length > 0, `${entry.path} has no methods`);
    assert.ok(!("handlers" in entry), `${entry.path} leaks Rust module paths in handlers`);
    for (const method of entry.methods) {
      const where = `${entry.path} ${method}`;
      assert.ok(methods.includes(method), `${where} has an unknown method ${method}`);
      const description = entry.descriptions?.[method];
      assert.ok(description && description.length > 10, `${where} has no hand-written description`);
      assert.equal(restatesDescription(description, entry.path), false, `${where} restates its method, path or type: ${description}`);
      assert.ok(entry.requestType[method], `${where} has no request type`);
      assert.ok(entry.responseType[method], `${where} has no response type`);
      assert.ok(Array.isArray(entry.queryParams[method]), `${where} has no query params list`);
    }
    const expected = [...entry.path.matchAll(/\{(\w+)\}/g)].map((m) => m[1]);
    assert.deepEqual(entry.pathParams.map((p) => p.name), expected, `${entry.path} has the wrong path params`);
  }
  assert.equal(at("/api/v1/lights/flash").area, "Devices and lights", "the flash route sits with the lights");
  assert.ok(doc.entries.some((e) => e.area === "Diagnostics"), "the Diagnostics area is missing");
});

test("no description carries an issue number, roadmap code or Rust name", () => {
  const banned = /#\d+|\bG[0-9]\b|::|PREVIEW_MAX_FPS|GROUP_PROPS|\(0\.\d+\.\d+\)|spine/i;
  for (const entry of doc.entries) {
    for (const [method, text] of Object.entries(entry.descriptions)) {
      assert.doesNotMatch(text, banned, `${entry.path} ${method} has internal wording: ${text}`);
    }
  }
});

test("every route and method has one curl example with the listener caveat and the right auth", () => {
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      const example = entry.examples?.[method];
      assert.ok(example, `${entry.path} ${method} has no example`);
      assert.match(example, /^# [^\n]+\ncurl(?: -X [A-Z]+)? /, `${entry.path} ${method} has no curl command`);
      assert.match(example, /^# [^\n]*Verify the listener[^\n]*\n/, `${entry.path} ${method} has no plain listener note`);
      assert.doesNotMatch(example, /\]\(/, `${entry.path} ${method} has a markdown link in its example`);
      const websocket = entry.path.startsWith("/api/v1/preview");
      if (entry.path === "/api/v1/status" || websocket) {
        assert.doesNotMatch(example, /Authorization: Bearer/, `${entry.path} ${method} must not take a bearer header`);
      } else {
        assert.match(example, /-H "Authorization: Bearer \$TOKEN"/, `${entry.path} ${method} has no bearer header`);
      }
      if (websocket) {
        assert.match(example, /token=\$TOKEN/, `${entry.path} ${method} has no ?token= value`);
        assert.doesNotMatch(example, /-X GET/, `${entry.path} ${method} names GET on a WebSocket URL`);
      }
      const body = /-d '([^']+)'/.exec(example)?.[1];
      if (body) {
        assert.doesNotMatch(body, /undefined|\[object Object\]/, `${entry.path} ${method} has a broken body example`);
        assert.match(example, /-H "Content-Type: application\/json"/, `${entry.path} ${method} sends a body without the JSON content type`);
        assert.doesNotMatch(body, /"props":"value"/, `${entry.path} ${method} treats props as a string`);
      }
    }
  }
  const output = at("/api/v1/output").examples.PATCH;
  const body = JSON.parse(/-d '([^']+)'/.exec(output)[1]);
  assert.ok(body.brightness > 0, "the output patch example has a bright value");
});

/** Route and method pairs whose handler takes a Json<...> body, read from the handler source. */
function handlerBodyRoutes() {
  const modSrc = fs.readFileSync(path.join(repo, "crates/canvas-daemon/src/api/mod.rs"), "utf8");
  const spine = fs.readFileSync(path.join(repo, "crates/canvas-proto/src/api.rs"), "utf8");
  const pathByConst = new Map([...spine.matchAll(/pub const (\w+): &str = "([^"]+)";/g)].map((m) => [m[1], m[2]]));
  const apiDir = path.join(repo, "crates/canvas-daemon/src/api");
  const sources = new Map(fs.readdirSync(apiDir).filter((f) => f.endsWith(".rs"))
    .map((f) => [path.basename(f, ".rs"), fs.readFileSync(path.join(apiDir, f), "utf8")]));
  const found = new Map();
  const router = modSrc.slice(modSrc.indexOf("pub fn router("));
  let pos = 0;
  while ((pos = router.indexOf(".route(", pos)) !== -1) {
    const open = pos + ".route".length;
    const close = matchBracket(router, open, "(", ")");
    const call = router.slice(open + 1, close);
    pos = close + 1;
    let depth = 0, comma = -1;
    for (let i = 0; i < call.length; i++) {
      if ("(<".includes(call[i])) depth++;
      else if (")>".includes(call[i])) depth--;
      else if (call[i] === "," && depth === 0) { comma = i; break; }
    }
    if (comma === -1) continue;
    const expr = call.slice(0, comma).trim();
    const routePath = expr.startsWith("routes::") ? pathByConst.get(expr.slice(8)) : expr.replace(/^"|"$/g, "");
    if (!routePath) continue;
    for (const m of call.slice(comma + 1).matchAll(/\b(get|post|put|patch|delete)\s*\(\s*([\w:]+)\s*\)/g)) {
      const [module, fn] = m[2].split("::");
      const src = sources.get(module) || "";
      const start = src.indexOf(`fn ${fn}(`);
      if (start === -1) continue;
      const openParen = start + `fn ${fn}`.length;
      const signature = src.slice(start, matchBracket(src, openParen, "(", ")") + 1);
      if (/Json\s*<|axum::body::Bytes|serde_json::from_slice/.test(signature)) {
        const extracted = /Json\s*<\s*([\w:]+)\s*>/.exec(signature)?.[1]?.split("::").pop();
        found.set(`${m[1].toUpperCase()} ${routePath}`, extracted || "");
      }
    }
  }
  return found;
}

test("every handler that takes a JSON body has a -d example with the content type", () => {
  const bodies = handlerBodyRoutes();
  const published = new Set();
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      const type = entry.requestType[method];
      if (type !== "none" && type !== "WebSocket (binary frames)") published.add(`${method} ${entry.path}`);
    }
  }
  assert.deepEqual([...bodies.keys()].sort(), [...published].sort(), "the handler body routes and the published request types disagree");
  let checked = 0;
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      const example = entry.examples[method];
      if (!bodies.has(`${method} ${entry.path}`)) continue;
      checked++;
      assert.match(example, /-d '[^']*'/, `${entry.path} ${method} takes a body but has no -d example`);
      assert.match(example, /-H "Content-Type: application\/json"/, `${entry.path} ${method} has no JSON content type header`);
    }
  }
  assert.equal(checked, bodies.size, "a handler with a JSON body has no reference entry");
});

test("a named spine request type is published as that type, never as a hand shape", () => {
  const protoDir = path.join(repo, "app", "web", "src", "proto");
  const spineTypes = new Set(fs.readdirSync(protoDir).filter((f) => f.endsWith(".ts")).map((f) => f.slice(0, -3)));
  // These two handlers read a plain value and validate it into the named type. They carry no
  // Json<NamedType> in their signature, so name them explicitly.
  const validated = new Map([
    ["PUT /api/v1/settings", "Settings"],
    ["PUT /api/v1/fans/headers/{h}/availability", "PutFanHeaderAvailability"],
  ]);
  for (const [key, extracted] of handlerBodyRoutes()) {
    if (spineTypes.has(extracted)) validated.set(key, extracted);
  }
  for (const [key, type] of validated) {
    const [method, ...rest] = key.split(" ");
    const routePath = rest.join(" ");
    const entry = at(routePath);
    assert.equal(entry.requestType[method], type, `${key} must publish ${type}`);
    assert.equal(entry.requestShape?.[method]?.label, type, `${key} must publish the shape of ${type}`);
    assert.deepEqual(
      (entry.requestShape?.[method]?.fields || []).map((f) => f.name),
      (doc.types[type]?.fields || []).map((f) => f.name),
      `${key} publishes a shape that does not match the ${type} table`,
    );
    assert.ok(doc.types[type], `the type table has no ${type}`);
  }
  assert.ok(validated.size >= 3, "the named-type guard found too few routes");
});

test("every route publishes the shape of its request and of its answer", () => {
  let requests = 0;
  let answers = 0;
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      const request = entry.requestType[method];
      const response = entry.responseType[method];
      if (request && request !== "none") {
        const shape = entry.requestShape?.[method];
        assert.ok(shape, `${entry.path} ${method} has no request shape`);
        assert.equal(shape.label, request, `${entry.path} ${method} has the wrong request shape label`);
        requests++;
      }
      if (response && response !== "none" && response !== "WebSocket (binary frames)") {
        const shape = entry.responseShape?.[method];
        assert.ok(shape, `${entry.path} ${method} has no response shape`);
        assert.equal(shape.label, response, `${entry.path} ${method} has the wrong response shape label`);
        answers++;
      }
    }
  }
  assert.ok(requests >= 40, `only ${requests} request shapes were published`);
  assert.ok(answers >= 85, `only ${answers} response shapes were published`);
});

test("an import example's file name ends in .html", () => {
  const entry = at("/api/v1/effects/import");
  const body = /-d '([^']+)'/.exec(entry.examples.POST)[1];
  const name = JSON.parse(body).file_name;
  assert.match(name, /\.html$/, `the import file name ${name} does not end in .html`);
});

test("no reader-facing type prints a Rust wrapper, generic or issue number", () => {
  const raw = fs.readFileSync(jsonPath, "utf8");
  for (const banned of [/serde_json/, /crates\//, /Vec</, /Option</, /"Response"/]) {
    assert.doesNotMatch(raw, banned, `api-reference.json contains ${banned}`);
  }
  const thumbnail = at("/api/v1/effects/{id}/thumbnail");
  assert.equal(thumbnail.responseType.GET, "Binary thumbnail", "the thumbnail answer is binary bytes");
  assert.match(thumbnail.descriptions.GET, /little-endian/i, "the thumbnail description omits the binary layout");
  assert.match(thumbnail.descriptions.GET, /octet-stream/i, "the thumbnail description omits the content type");
});

test("every JSON object type has a hand-written shape with field, type and meaning", () => {
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      for (const [typeKey, shapeKey] of [["requestType", "requestShape"], ["responseType", "responseShape"]]) {
        if (entry[typeKey][method] !== "JSON object") continue;
        const shape = entry[shapeKey]?.[method];
        assert.ok(shape, `${entry.path} ${method} has no hand-written ${shapeKey}`);
        assert.equal(shape.label, "JSON object", `${entry.path} ${method} has the wrong shape label`);
        assert.ok(shape.fields.length, `${entry.path} ${method} has no fields in its hand shape`);
        for (const field of shape.fields) {
          assert.ok(field.name && field.type && field.meaning, `${entry.path} ${method} has an incomplete field: ${JSON.stringify(field)}`);
        }
      }
    }
  }
  assert.ok(fresh.entries.length === doc.entries.length, "fresh generation drifted from the committed entries");
});

test("answer shapes carry the fields read from the generated TypeScript", () => {
  assert.ok(doc.types.Status, "no Status shape");
  const names = doc.types.Status.fields.map((f) => f.name);
  assert.ok(names.includes("version") && names.includes("power") && names.includes("onboarding_complete"), "Status fields were not read from Status.ts");
  const optional = doc.types.Status.fields.find((f) => f.name === "active_scene");
  assert.equal(optional?.optional, true, "an optional Status field was marked required");
  assert.match(doc.types.Props?.note || "", /map of property id to PropValue/i, "Props is not described as a map of property id to PropValue");
  const used = new Set();
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      used.add(entry.requestType[method]);
      used.add(entry.responseType[method]);
    }
  }
  for (const name of used) {
    if (["none", "WebSocket (binary frames)", "", "JSON object", "Binary thumbnail"].includes(name)) continue;
    assert.ok(doc.types[name], `no shape for ${name}`);
    assert.ok(Array.isArray(doc.types[name].fields), `${name} has no fields list`);
  }
});

test("GET /api/v1/status answers the Status body", () => {
  assert.equal(at("/api/v1/status").responseType.GET, "Status");
  assert.equal(fresh.entries.find((e) => e.path === "/api/v1/status").responseType.GET, "Status");
});

test("the committed JSON matches a fresh generation", () => {
  assert.equal(committed, serialize(fresh), "api-reference.json is stale; run node site/scripts/gen-api-reference.mjs");
  assert.equal(fresh.entries.length, spineRoutes().length + 1);
});

/** The named types inside a reader-facing type label, independent of the generator's copy. */
function namedIn(label) {
  const s = String(label).trim();
  const inner = /^array of (.+)$/.exec(s)
    || /^(.+) or null$/.exec(s)
    || /^an object keyed by (?:text|number), (?:holding|with) (.+?)(?: values)?$/.exec(s);
  if (inner) return namedIn(inner[1]);
  const out = new Set();
  for (const part of s.includes(" or ") ? s.split(" or ") : [s]) {
    const name = part.trim().split("::").pop();
    if (/^[A-Z]\w*$/.test(name)) out.add(name);
  }
  return out;
}

test("every referenced type name has a definition", () => {
  const defined = new Set(Object.keys(doc.types));
  const missing = new Set();
  const check = (label, where) => {
    for (const name of namedIn(label)) {
      if (!defined.has(name)) missing.add(`${name} (from ${where}: ${label})`);
    }
  };
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      const where = `${entry.path} ${method}`;
      check(entry.requestType[method], `${where} request`);
      check(entry.responseType[method], `${where} response`);
      check(entry.queryType?.[method], `${where} query`);
      for (const key of ["requestShape", "responseShape"]) {
        for (const field of entry[key]?.[method]?.fields || []) check(field.type, `${where} ${key}.${field.name}`);
      }
    }
  }
  for (const [name, def] of Object.entries(doc.types)) {
    for (const field of def.fields || []) check(field.type, `types.${name}.${field.name}`);
  }
  assert.deepEqual([...missing].sort(), [], `referenced types with no definition:\n${[...missing].sort().join("\n")}`);
});

test("published examples use neutral identifiers and no local paths", () => {
  const raw = fs.readFileSync(jsonPath, "utf8");
  // The config bundle's leftover light names the one device the daemon never drives, so the light
  // delete example answers 2xx as published. Every other vendor id stays out of the examples.
  const notDriven = "openrgb:HID:VID_3151&PID_4015";
  const withoutNotDriven = raw.split(notDriven).join("");
  const localPath = new RegExp(`\\bC:${String.fromCharCode(92)}+${["Use", "rs"].join("")}\\b`, "i");
  for (const [pattern, label] of [
    [/fake:/, "a fake: id"],
    [/chione/i, "a product name"],
    [/VID_/, "a real vendor id"],
    [localPath, "a user folder path"],
    [/\bI\b|\bmy\b/, "first person"],
  ]) {
    assert.doesNotMatch(withoutNotDriven, pattern, `api-reference.json contains ${label}`);
  }
  const text = JSON.stringify(doc);
  assert.match(text, /openrgb:[0-9a-f]{8}/, "no invented OpenRGB device id in the examples");
  for (const placeholder of ["<keep_presets>", "<force>", "<epoch>", "<lease>", "<token>", "<stack>"]) {
    assert.ok(!text.includes(placeholder), `a query example still pastes ${placeholder}`);
  }
});

test("reader text carries no internal wording or mapped-type notation", () => {
  const banned = /spine|daemon's router|main task|WebView|shim|\{ \[key in|lit-share|recipe/i;
  const check = (text, where) => assert.doesNotMatch(String(text), banned, `${where} carries internal wording`);
  for (const entry of doc.entries) {
    for (const [method, description] of Object.entries(entry.descriptions)) check(description, `${entry.path} ${method}`);
    for (const example of Object.values(entry.examples)) check(example, `${entry.path} example`);
    for (const [direction, key] of [["request", "requestShape"], ["response", "responseShape"]]) {
      for (const shape of Object.values(entry[key] || {})) {
        check(shape.note, `${entry.path} ${direction} note`);
        for (const field of shape.fields || []) check(field.meaning, `${entry.path} ${direction}.${field.name}`);
      }
    }
    for (const params of Object.values(entry.queryParams)) for (const p of params) check(p.meaning, `${entry.path} ?${p.name}`);
  }
  for (const [name, def] of Object.entries(doc.types)) {
    check(def.note, `types.${name} note`);
    for (const field of def.fields || []) check(field.meaning, `types.${name}.${field.name}`);
    for (const value of def.values || []) check(value.meaning, `types.${name}.${value.value}`);
  }
});

test("query parameters carry a value, a type and a meaning", () => {
  let checked = 0;
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      for (const p of entry.queryParams[method] || []) {
        const where = `${entry.path} ${method} ?${p.name}`;
        assert.ok(p.name, `${where} has no name`);
        assert.ok(p.type, `${where} has no type`);
        assert.ok(p.meaning && p.meaning.length > 10, `${where} has no plain meaning`);
        // The lease is the one value a reader pastes from an earlier answer's body.
        const pasted = p.name === "lease" && String(p.value) === "<lease_id>";
        assert.ok(p.value && (pasted || !/^<.+>$/.test(String(p.value))), `${where} pastes a placeholder`);
        if (pasted) assert.match(p.meaning, /lease_id returned by the audio test PUT/, `${where} does not explain the pasted value`);
        checked++;
      }
    }
  }
  assert.ok(checked >= 15, `only ${checked} query parameters were documented`);
});

/** The query parameters the handlers read, read from the handler sources alone. */
function handlerQueryParams() {
  const modSrc = fs.readFileSync(path.join(repo, "crates/canvas-daemon/src/api/mod.rs"), "utf8");
  const spine = fs.readFileSync(path.join(repo, "crates/canvas-proto/src/api.rs"), "utf8");
  const pathByConst = new Map([...spine.matchAll(/pub const (\w+): &str = "([^"]+)";/g)].map((m) => [m[1], m[2]]));
  const apiDir = path.join(repo, "crates/canvas-daemon/src/api");
  const files = fs.readdirSync(apiDir).filter((f) => f.endsWith(".rs"));
  const sources = new Map(files.map((f) => [path.basename(f, ".rs"), fs.readFileSync(path.join(apiDir, f), "utf8")]));
  const everyStruct = [spine, ...sources.values()].join("\n");
  const found = new Set();
  const router = modSrc.slice(modSrc.indexOf("pub fn router("));
  let pos = 0;
  while ((pos = router.indexOf(".route(", pos)) !== -1) {
    const open = pos + ".route".length;
    const close = matchBracket(router, open, "(", ")");
    const call = router.slice(open + 1, close);
    pos = close + 1;
    let depth = 0, comma = -1;
    for (let i = 0; i < call.length; i++) {
      if ("(<".includes(call[i])) depth++;
      else if (")>".includes(call[i])) depth--;
      else if (call[i] === "," && depth === 0) { comma = i; break; }
    }
    if (comma === -1) continue;
    const expr = call.slice(0, comma).trim();
    const routePath = expr.startsWith("routes::") ? pathByConst.get(expr.slice(8)) : expr.replace(/^"|"$/g, "");
    if (!routePath) continue;
    for (const m of call.slice(comma + 1).matchAll(/\b(get|post|put|patch|delete)\s*\(\s*([\w:]+)\s*\)/g)) {
      const [module, fn] = m[2].split("::");
      const src = sources.get(module) || "";
      const start = src.indexOf(`fn ${fn}(`);
      if (start === -1) continue;
      const openParen = start + `fn ${fn}`.length;
      const signature = src.slice(start, matchBracket(src, openParen, "(", ")") + 1);
      const names = new Set();
      const query = /Query\s*<\s*([\w:]+)\s*>/.exec(signature);
      if (query) {
        const struct = query[1].split("::").pop();
        const block = new RegExp(`pub\\s+struct\\s+${struct}\\s*\\{([\\s\\S]*?)\\n\\}`).exec(everyStruct);
        if (block) for (const f of block[1].matchAll(/^\s*pub\s+(\w+)\s*:/gm)) names.add(f[1]);
      }
      // A raw query is parsed into flags by parse_flag, here or in a helper of the same file.
      if (/RawQuery\s*\(/.test(signature)) {
        for (const f of src.matchAll(/parse_flag\s*\(\s*[^,]+,\s*"([^"]+)"/g)) names.add(f[1]);
      }
      for (const name of names) found.add(`${m[1].toUpperCase()} ${routePath} ?${name}`);
    }
  }
  return found;
}

test("every query parameter the handlers read is published", () => {
  const found = handlerQueryParams();
  const published = new Set();
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      for (const p of entry.queryParams[method] || []) published.add(`${method} ${entry.path} ?${p.name}`);
    }
  }
  const missing = [...found].filter((key) => !published.has(key)).sort();
  assert.deepEqual(missing, [], `query parameters the handlers read that the reference leaves out:\n${missing.join("\n")}`);
  assert.ok(found.size >= 15, `only ${found.size} query parameters were found in the handlers`);
  assert.ok(published.has("GET /api/v1/devices ?rescan"), "the devices rescan flag is not published");
  assert.ok(published.has("GET /api/v1/peripherals/{id} ?fresh"), "the peripherals fresh flag is not published");
});

test("every path parameter says what it names", () => {
  let checked = 0;
  for (const entry of doc.entries) {
    for (const p of entry.pathParams) {
      const where = `${entry.path} {${p.name}}`;
      assert.ok(p.name, `${where} has no name`);
      assert.ok(p.type, `${where} has no reader-facing type`);
      assert.ok(p.meaning && p.meaning.length > 10, `${where} has no plain meaning`);
      checked++;
    }
  }
  assert.ok(checked >= 39, `only ${checked} path parameters were documented`);
});

test("every field a reader can put in a request body has a meaning", () => {
  const wanted = new Set();
  for (const entry of doc.entries) {
    for (const method of entry.methods) wanted.add(entry.requestType[method]);
  }
  const seen = new Set();
  const queue = [...wanted];
  while (queue.length) {
    const name = queue.shift();
    if (seen.has(name) || !doc.types[name]) continue;
    seen.add(name);
    for (const field of doc.types[name].fields || []) {
      for (const ref of namedIn(field.type)) queue.push(ref);
    }
  }
  let checked = 0;
  for (const name of seen) {
    for (const field of doc.types[name].fields || []) {
      assert.ok(field.meaning && field.meaning.length > 4,
        `${name}.${field.name} has no plain meaning: ${JSON.stringify(field)}`);
      checked++;
    }
  }
  assert.ok(checked >= 290, `only ${checked} request-body fields were documented`);
  assert.deepEqual(fresh.entries.length, doc.entries.length, "fresh generation drifted from the committed entries");
});

test("the preview sockets describe their frames", () => {
  for (const path of ["/api/v1/preview", "/api/v1/preview/group"]) {
    assert.match(at(path).descriptions.GET, /16-bit little-endian width/, `${path} omits the frame layout`);
    assert.match(at(path).descriptions.GET, /RGB triples/, `${path} omits the pixel bytes`);
  }
  const lights = at("/api/v1/preview/lights").descriptions.GET;
  assert.match(lights, /opening message/, "the light feed omits its first message");
  assert.match(lights, /16-bit little-endian light index/, "the light feed omits its frame layout");
});

test("every published field and enum value carries a meaning", () => {
  let fields = 0, values = 0;
  for (const [name, def] of Object.entries(doc.types)) {
    for (const field of def.fields || []) {
      assert.ok(field.meaning && field.meaning.length > 4,
        `${name}.${field.name} has no meaning: ${JSON.stringify(field)}`);
      fields++;
    }
    for (const value of def.values || []) {
      assert.ok(value.meaning && value.meaning.length > 4,
        `${name}.${value.value} has no meaning`);
      values++;
    }
  }
  assert.ok(fields > 500, `only ${fields} fields have meanings`);
  assert.ok(values > 100, `only ${values} enum values have meanings`);
});

test("meaning tables are keyed by type and field, never by a bare name", () => {
  const keys = meaningKeys();
  for (const [kind, list] of Object.entries(keys)) {
    for (const key of list) {
      assert.match(key, /\./, `${kind} meaning key ${key} is a bare field name`);
    }
  }
  assert.ok(keys.fields.length >= 100, `only ${keys.fields.length} per-field meanings`);
  assert.ok(keys.values.length >= 100, `only ${keys.values.length} per-value meanings`);
});

test("no hand-written table names the same key twice", () => {
  const src = fs.readFileSync(path.join(site, "scripts", "gen-api-reference.mjs"), "utf8");
  for (const table of ["FIELD_MEANINGS", "ENUM_VALUE_MEANINGS"]) {
    const block = new RegExp(`const ${table} = \\{([\\s\\S]*?)\\n\\};`).exec(src);
    assert.ok(block, `the ${table} table is missing from the generator`);
    const keys = [...block[1].matchAll(/^\s*"([^"]+)":/gm)].map((m) => m[1]);
    const seen = new Set();
    const dupes = [...new Set(keys.filter((key) => (seen.has(key) ? true : (seen.add(key), false))))];
    assert.deepEqual(dupes, [], `${table} names the same key twice: ${dupes.join(", ")}`);
  }
});

test("a meaning's leading type word agrees with the field type", () => {
  for (const [name, def] of Object.entries(doc.types)) {
    for (const field of def.fields || []) {
      const where = `${name}.${field.name}`;
      const t = String(field.type);
      const m = String(field.meaning);
      if (/^array of /.test(t)) {
        assert.doesNotMatch(m, /^(boolean:|number:|string: the id|a map from)/i,
          `${where} is an array described as ${m}`);
      }
      if (/^(boolean|boolean or null)$/.test(t)) {
        assert.doesNotMatch(m, /^(a list of|array of|a map from|number:|string:)/i,
          `${where} is a boolean described as ${m}`);
      }
      if (/^(number|integer)/.test(t)) {
        assert.doesNotMatch(m, /^(a list of|boolean:|a map from|string:)/i,
          `${where} is a number described as ${m}`);
      }
      if (/^map of /.test(t)) {
        assert.doesNotMatch(m, /^(boolean:|number:|a list of|string: the id)/i,
          `${where} is a map described as ${m}`);
      }
      if (/^string/.test(t)) {
        assert.doesNotMatch(m, /^(a list of|boolean:|a map from|array of)/i,
          `${where} is a string described as ${m}`);
      }
    }
  }
});

test("every string enum publishes one meaning per value", () => {
  let checked = 0;
  for (const [name, def] of Object.entries(doc.types)) {
    const listed = /^One of: (.*)\.$/.exec(def.note || "");
    if (!listed) continue;
    const values = listed[1].split(", ").map((v) => v.replace(/^"|"$/g, ""));
    const published = (def.values || []).map((v) => v.value);
    assert.deepEqual(published, values, `${name} publishes the wrong set of values`);
    checked++;
  }
  assert.ok(checked >= 25, `only ${checked} string enums were checked`);
});

/** The function words a meaning may not end on. */
const DANGLING_WORDS = ["a", "an", "the", "in", "of", "is", "see", "and", "or"];

/** The words no published meaning may carry. The wire values in code spans are left out. */
const BANNED_WORDS = /\bspine\b|\bpipe\b|\brouter\b|main task|WebView|\bshim\b|lit-share|\brecipe\b|\bowner\b|Boot color|instead of|rather than|\bcolor\b/i;

/** Everything wrong with one meaning, as sentences a reader of the report can act on. */
function meaningProblems(text, where) {
  const problems = [];
  const meaning = String(text ?? "").trim();
  if (!meaning) return [`${where} has no meaning`];
  const words = meaning.split(/\s+/);
  if (words.length < 3) problems.push(`${where} is shorter than three words: ${meaning}`);
  if (/^[=,]/.test(meaning)) problems.push(`${where} starts with a stray mark: ${meaning}`);
  if (/^(string|number|boolean|integer|text|JSON object)\s*:/i.test(meaning)) {
    problems.push(`${where} starts with its type: ${meaning}`);
  }
  // A word repeated inside one sentence is a slip; the same word in two sentences is not.
  for (const sentence of meaning.split(/\.\s+/)) {
    const said = sentence.split(/\s+/);
    for (let i = 1; i < said.length; i++) {
      const before = said[i - 1].toLowerCase().replace(/[^a-z]/g, "");
      const after = said[i].toLowerCase().replace(/[^a-z]/g, "");
      if (before && before === after) problems.push(`${where} repeats the word "${before}": ${meaning}`);
    }
  }
  const last = words[words.length - 1].toLowerCase().replace(/[^a-z]/g, "");
  if (DANGLING_WORDS.includes(last)) problems.push(`${where} ends on the word "${last}": ${meaning}`);
  const prose = meaning.replace(/`[^`]*`/g, "");
  if (BANNED_WORDS.test(prose)) problems.push(`${where} carries a banned word: ${meaning}`);
  return problems;
}

test("every published meaning reads as a sentence", () => {
  const problems = [];
  let checked = 0;
  const check = (text, where) => {
    problems.push(...meaningProblems(text, where));
    checked++;
  };
  for (const [name, def] of Object.entries(doc.types)) {
    for (const field of def.fields || []) check(field.meaning, `types.${name}.${field.name}`);
    for (const value of def.values || []) check(value.meaning, `types.${name}.${value.value}`);
  }
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      for (const p of entry.pathParams || []) check(p.meaning, `${entry.path} {${p.name}}`);
      for (const p of entry.queryParams[method] || []) check(p.meaning, `${entry.path} ?${p.name}`);
      for (const direction of ["requestShape", "responseShape"]) {
        for (const field of entry[direction]?.[method]?.fields || []) {
          check(field.meaning, `${entry.path} ${method} ${direction}.${field.name}`);
        }
      }
    }
  }
  assert.deepEqual(problems, [], `meanings that do not read as sentences:\n${problems.join("\n")}`);
  assert.ok(checked >= 900, `only ${checked} meanings were checked`);
});

/** The Rust structs behind the published types, read from the protocol and the daemon's own. */
const rustByName = rustStructs();

test("a request field with a serde default is shown optional", () => {
  let checked = 0;
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      const type = entry.requestType[method];
      if (!type || type === "none" || type === "JSON object") continue;
      const rust = rustByName.get(type);
      if (!rust) continue;
      const shape = entry.requestShape?.[method];
      assert.ok(shape, `${entry.path} ${method} has no request shape`);
      for (const field of rust.fields.values()) {
        // A field the API does not carry is not a field a reader may leave out.
        if (field.flatten || field.skip) continue;
        if (!field.default && !field.option && !rust.default) continue;
        const published = (shape.fields || []).find((f) => f.name === field.published);
        assert.ok(published, `${entry.path} ${method} has no field ${field.published} to leave out`);
        assert.equal(
          published.optional,
          true,
          `${entry.path} ${method} ${field.published} may be left out but is not marked optional`,
        );
        checked++;
      }
    }
  }
  assert.ok(checked >= 15, `only ${checked} request fields were checked`);
});

/** The source of the handler a route and method call, read from the daemon's own files. */
function handlerSource(method, routePath) {
  const modSrc = fs.readFileSync(path.join(repo, "crates/canvas-daemon/src/api/mod.rs"), "utf8");
  const spine = fs.readFileSync(path.join(repo, "crates/canvas-proto/src/api.rs"), "utf8");
  const pathByConst = new Map(
    [...spine.matchAll(/pub const (\w+): &str = "([^"]+)";/g)].map((m) => [m[1], m[2]]),
  );
  const apiDir = path.join(repo, "crates/canvas-daemon/src/api");
  const sources = new Map(
    fs.readdirSync(apiDir)
      .filter((f) => f.endsWith(".rs"))
      .map((f) => [path.basename(f, ".rs"), fs.readFileSync(path.join(apiDir, f), "utf8")]),
  );
  const router = modSrc.slice(modSrc.indexOf("pub fn router("));
  let pos = 0;
  while ((pos = router.indexOf(".route(", pos)) !== -1) {
    const open = pos + ".route".length;
    const close = matchBracket(router, open, "(", ")");
    const call = router.slice(open + 1, close);
    pos = close + 1;
    let depth = 0;
    let comma = -1;
    for (let i = 0; i < call.length; i++) {
      if ("(<".includes(call[i])) depth++;
      else if (")>".includes(call[i])) depth--;
      else if (call[i] === "," && depth === 0) { comma = i; break; }
    }
    if (comma === -1) continue;
    const expr = call.slice(0, comma).trim();
    const route = expr.startsWith("routes::")
      ? pathByConst.get(expr.slice(8))
      : expr.replace(/^"|"$/g, "");
    for (const m of call.slice(comma + 1).matchAll(/\b(get|post|put|patch|delete)\s*\(\s*([\w:]+)\s*\)/g)) {
      if (m[1].toUpperCase() !== method || route !== routePath) continue;
      const [module, fn] = m[2].split("::");
      const src = sources.get(module) || "";
      const start = src.indexOf(`fn ${fn}(`);
      if (start === -1) return "";
      const openParen = start + `fn ${fn}`.length;
      const bodyStart = src.indexOf("{", matchBracket(src, openParen, "(", ")"));
      return src.slice(start, matchBracket(src, bodyStart, "{", "}") + 1);
    }
  }
  return "";
}

/** The keys one answer carries, read from the handler's own return expression. */
function answerKeys(src, note) {
  const keys = new Set();
  // The object a handler builds with json!.
  for (const m of src.matchAll(/json!\s*\(\s*\{([\s\S]*?)\}\s*\)/g)) {
    for (const key of m[1].matchAll(/(?:^|[,{])\s*"([^"]+)"\s*:/g)) keys.add(key[1]);
  }
  // The struct literal a handler returns: Json(Name { field: value }).
  for (const m of src.matchAll(/Json\s*\(\s*[A-Za-z_]\w*\s*\{([\s\S]*?)\}/g)) {
    for (const key of m[1].matchAll(/(?:^|[,{])\s*([a-z_]\w*)\s*:/g)) keys.add(key[1]);
  }
  // The named type a handler hands to serde: to_value(&status). The local's type is read from
  // the note, which points at the type the answer carries.
  for (const m of src.matchAll(/to_value\s*\(\s*&\s*([A-Za-z_]\w*)\s*\)/g)) {
    const named = rustByName.get(m[1]);
    if (named) {
      for (const field of named.fields.values()) keys.add(field.published);
      continue;
    }
    for (const type of (note || "").matchAll(/`([A-Z]\w*)`/g)) {
      const rust = rustByName.get(type[1]);
      if (!rust) continue;
      for (const field of rust.fields.values()) keys.add(field.published);
    }
  }
  return keys;
}

test("a hand shape carries the fields the handler sends", () => {
  let checked = 0;
  for (const entry of doc.entries) {
    for (const method of entry.methods) {
      if (entry.responseType[method] !== "JSON object") continue;
      const shape = entry.responseShape[method];
      const src = handlerSource(method, entry.path);
      assert.ok(src, `${entry.path} ${method} has no handler source`);
      const sent = answerKeys(src, shape.note);
      assert.ok(sent.size, `${entry.path} ${method} has no answer keys in its handler`);
      const published = (shape.fields || []).map((f) => f.name);
      for (const name of published) {
        assert.ok(
          sent.has(name),
          `${entry.path} ${method} publishes ${name}, which the handler does not send`,
        );
      }
      for (const name of sent) {
        if (published.includes(name)) continue;
        // A key the handler sends and the shape leaves out: the note under the shape says so.
        assert.ok(
          (shape.note || "").includes(name),
          `${entry.path} ${method} sends ${name}, which the shape leaves out and its note does not name`,
        );
      }
      checked++;
    }
  }
  assert.ok(checked >= 21, `only ${checked} hand shapes were checked`);
});

/* ------------------------------------------------------------------ the CLI reference */

const cliPath = path.join(site, "content", "docs", "cli-reference.json");
const cliCommitted = fs.readFileSync(cliPath, "utf8").replace(/\r\n/g, "\n");
const cliDoc = JSON.parse(cliCommitted);
const cliFresh = generateCli();

test("the committed CLI reference matches a fresh generation from the clap definitions", () => {
  assert.equal(cliCommitted, serializeCli(cliFresh), "cli-reference.json is stale; run node site/scripts/gen-cli-reference.mjs");
  assert.equal(Object.keys(cliDoc.commands).length, 33, "the CLI reference does not cover every command page");
});

test("every command flag has its own meaning from the clap doc comment and default", () => {
  const fallback = /see the cli reference|global flag|not documented|unknown|see the reference/i;
  let flags = 0;
  for (const [slug, command] of Object.entries(cliDoc.commands)) {
    assert.ok(Array.isArray(command.flags), `${slug} has no flags list`);
    const byMeaning = new Map();
    for (const flag of command.flags) {
      const where = `${slug} ${flag.flag}`;
      assert.match(flag.flag, /^--[a-z][\w-]*$/, `${where} is not a long flag`);
      assert.ok(flag.meaning && flag.meaning.length > 10, `${where} has no meaning`);
      assert.doesNotMatch(flag.meaning, fallback, `${where} carries a fallback sentence: ${flag.meaning}`);
      assert.doesNotMatch(flag.meaning, /\bglobal\b/i, `${where} is called global: ${flag.meaning}`);
      if (byMeaning.has(flag.meaning)) {
        assert.fail(`${where} repeats the meaning of ${byMeaning.get(flag.meaning)}: ${flag.meaning}`);
      }
      byMeaning.set(flag.meaning, flag.flag);
      flags++;
    }
  }
  assert.equal(flags, 41, `only ${flags} command flags were documented`);
  const frames = cliDoc.commands["diag-dump-frames"].flags;
  const distinct = new Set(frames.filter((f) => ["--frames", "--fps", "--width", "--height"].includes(f.flag)).map((f) => f.meaning));
  assert.equal(distinct.size, 4, "the frame flags share one meaning");
  assert.match(frames.find((f) => f.flag === "--frames").meaning, /Defaults to 60/);
  assert.match(frames.find((f) => f.flag === "--width").meaning, /Defaults to 320/);
  const out = frames.find((f) => f.flag === "--out");
  assert.match(out.meaning, /frames/, "dump-frames --out does not say what it writes");
  assert.match(cliDoc.commands["effects-activate"].flags.find((f) => f.flag === "--group").meaning, /groups of the active scene/);
});

test("no published CLI meaning ends mid-word", () => {
  const findings = [];
  const check = (text, where) => {
    const meaning = String(text ?? "").trim();
    if (!/[.!?]$/.test(meaning)) findings.push(`${where} ends without sentence punctuation: ${meaning}`);
    if (/\.\.\.$/.test(meaning)) findings.push(`${where} was cut short: ${meaning}`);
  };
  for (const [slug, command] of Object.entries(cliDoc.commands)) {
    for (const flag of command.flags) check(flag.meaning, `${slug} ${flag.flag}`);
    for (const code of command.exitCodes) check(code.meaning, `${slug} exit ${code.code}`);
  }
  for (const flag of cliDoc.globalFlags) check(flag.meaning, `global ${flag.flag}`);
  assert.deepEqual(findings, [], `truncated CLI prose:\n${findings.join("\n")}`);
});

test("every command page has a human title and every flag a value placeholder", () => {
  for (const [slug, command] of Object.entries(cliDoc.commands)) {
    assert.match(command.title, /^[A-Z][a-z]+(?: [\w-]+){0,3}$/, `${slug} has no human title`);
    assert.doesNotMatch(command.title, /canvas-rgb/, `${slug} uses the command line as its title`);
    for (const flag of command.flags) {
      assert.ok(flag.value, `${slug} ${flag.flag} has no VALUE column`);
      assert.doesNotMatch(flag.value, /^$/, `${slug} ${flag.flag} has an empty VALUE column`);
    }
  }
  for (const flag of cliDoc.globalFlags) assert.ok(flag.value, `global ${flag.flag} has no VALUE column`);
});

test("only the two real global flags are called global", () => {
  assert.deepEqual(cliDoc.globalFlags.map((f) => f.flag), ["--json", "--api"]);
  for (const command of Object.values(cliDoc.commands)) {
    for (const flag of command.flags) {
      assert.doesNotMatch(flag.meaning, /global/i, `${flag.flag} is described as global`);
    }
  }
});

test("no-daemon commands carry no API-listener exit code", () => {
  const bin = fs.readFileSync(path.join(repo, "crates", "canvas-daemon", "src", "bin", "cli.rs"), "utf8");
  for (const [slug, handler] of [["effects-check", "effects_check"], ["diag-dump-frames", "diag_dump_frames"], ["diag-bundle", "diag_bundle"]]) {
    const at = bin.indexOf(`fn ${handler}(`);
    assert.notEqual(at, -1, `no handler ${handler} in bin/cli.rs`);
    assert.doesNotMatch(bin.slice(at, bin.indexOf("\n}", at)), /\bclient\s*\(/, `${slug} builds a daemon client`);
    const codes = cliDoc.commands[slug].exitCodes.map((c) => c.code);
    assert.deepEqual(codes, ["0", "1"], `${slug} has the wrong exit codes: ${codes}`);
  }
  for (const [slug, command] of Object.entries(cliDoc.commands)) {
    if (["effects-check", "diag-dump-frames", "diag-bundle"].includes(slug)) continue;
    assert.equal(command.needsDaemon, true, `${slug} is marked as needing no daemon`);
    assert.deepEqual(command.exitCodes.map((c) => c.code), ["0", "1", "2"], `${slug} has the wrong exit codes`);
    assert.match(command.exitCodes.find((c) => c.code === "2").meaning, /no listener/);
  }
});

test("failure documents are a --json condition in the CLI prose", () => {
  const cli = fs.readFileSync(path.join(site, "content", "docs", "cli.md"), "utf8");
  assert.match(cli, /With `--json`, a failed[^\n]+prints one document on stdout/);
  assert.match(cli, /Without `--json`, no command writes a failure document or an error body to stdout/);
  assert.doesNotMatch(cli, /all print `\{"saved": false/);
});

test("the group path parameters name the scene in the id parameter", () => {
  for (const routePath of [
    "/api/v1/scenes/{id}/groups/{group}/brightness",
    "/api/v1/scenes/{id}/groups/{group}/move",
    "/api/v1/scenes/{id}/groups/{group}/props",
  ]) {
    const group = at(routePath).pathParams.find((p) => p.name === "group");
    assert.match(group.meaning, /scene named by the `id` path parameter/, `${routePath} still names the active scene`);
    assert.match(group.meaning, /Any scene, active or not\./, `${routePath} does not state the scene choice affirmatively`);
    assert.doesNotMatch(group.meaning, /need not/, `${routePath} uses a negated-opposite phrase`);
  }
});

test("generated prose states its point affirmatively", () => {
  // A negated opposite makes a reader resolve a double negative. The four forms below are the
  // ones the docs have used; each has a direct affirmative wording.
  const banned = /\b(?:need not|needn't|not un\w*|not without|no small|no minor|not a small|not a minor|isn't wrong|is not wrong|hardly surprising)\b/i;
  const findings = [];
  const check = (text, where) => {
    if (banned.test(String(text ?? ""))) findings.push(`${where}: ${text}`);
  };
  const walk = (value, where) => {
    if (typeof value === "string") check(value, where);
    else if (Array.isArray(value)) value.forEach((v, i) => walk(v, `${where}[${i}]`));
    else if (value && typeof value === "object") {
      for (const [key, child] of Object.entries(value)) walk(child, `${where}.${key}`);
    }
  };
  walk(doc, "api-reference.json");
  walk(cliDoc, "cli-reference.json");
  const cliProse = fs.readFileSync(cliPath, "utf8").replace(/\r\n/g, "\n");
  for (const line of cliProse.split("\n")) check(line.replace(/^[#>*\s-]+/, ""), "cli.md");
  assert.deepEqual(findings, [], `negated-opposite phrasing in generated content:\n${findings.join("\n")}`);
});

test("published validation rules match the handlers", () => {
  const white = at("/api/v1/devices/{id}/white-balance").descriptions.PUT;
  assert.match(white, /0 to 1/);
  assert.match(white, /422/);
  const header = at("/api/v1/fans/headers/{h}").descriptions.PATCH;
  for (const phrase of ["floor", "422", "`fixed`", "`source`", "`curve`", "2 to 8"]) {
    assert.match(header, new RegExp(phrase.replace(/[`]/g, "")), `the fan header description omits ${phrase}`);
  }
  const sensor = at("/api/v1/diag/fail-sensor").descriptions.POST;
  for (const phrase of ["cpu", "gpu", "board:<name>"]) assert.match(sensor, new RegExp(phrase.replace(/[.:<>]/g, "\\$&")), `fail-sensor omits ${phrase}`);
  const access = at("/api/v1/peripherals/{id}/access").descriptions.PUT;
  assert.match(access, /`writes` without `reads` answers 422/);
});

test("WebSocket curl examples state the curl version and point at JavaScript", () => {
  for (const routePath of ["/api/v1/preview", "/api/v1/preview/group", "/api/v1/preview/lights"]) {
    const example = at(routePath).examples.GET;
    assert.match(example, /curl built with WebSocket support \(7\.86 or later\)/, `${routePath} has no curl caveat`);
    assert.match(example, /JavaScript sample is the main example/, `${routePath} does not point at JavaScript`);
  }
});
