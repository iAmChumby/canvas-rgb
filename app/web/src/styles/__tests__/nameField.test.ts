// The name field's sizing rules (styles/presets.css). jsdom does no layout, so the
// tests read the stylesheet itself. Comments are stripped first, and every rule's
// body is taken with matched braces, so an assertion only sees the declarations of
// the rule it names — never a declaration in a comment, in a later rule, or inside
// another rule's body (#108).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

// Line endings and comments are normalised out, so needles match on either and can
// not hide inside a comment.
const css = readFileSync(resolve(process.cwd(), "src/styles/presets.css"), "utf8")
  .replace(/\r\n/g, "\n")
  .replace(/\/\*[\s\S]*?\*\//g, "");

type Rule = { selector: string; body: string };

// Every rule of a stylesheet text, in order. A rule's selector is the text before its
// opening brace; its body runs to the brace that closes that same brace, so an
// @media block is one rule whose body holds the block's own inner rules.
function parseRules(text: string): Rule[] {
  const rules: Rule[] = [];
  let at = 0;
  while (at < text.length) {
    const open = text.indexOf("{", at);
    if (open === -1) break;
    let depth = 1;
    let close = open + 1;
    while (close < text.length && depth > 0) {
      if (text[close] === "{") depth += 1;
      else if (text[close] === "}") depth -= 1;
      close += 1;
    }
    rules.push({
      selector: oneLine(text.slice(at, open)),
      body: close <= text.length ? text.slice(open + 1, close - 1) : "",
    });
    at = close;
  }
  return rules;
}

// Whitespace collapsed, so a needle is written the way the selector reads.
function oneLine(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

// The body of the rule whose selector is exactly this text, or "" when absent.
function ruleBody(text: string, selector: string): string {
  const needle = oneLine(selector);
  const found = parseRules(text).find((rule) => rule.selector === needle);
  return found ? found.body : "";
}

const rowSelector = `
.preset-line .name-field,
.preset-save .name-field`;

describe("the name field rules", () => {
  it("the base rule fills whatever container it sits in, without leaning on being a flex item", () => {
    const base = ruleBody(css, ".name-field");
    expect(base).toMatch(/width:\s*100%/);
    // width: 0 beside flex sizes only a field that is a flex item of a row; anywhere
    // else it left the field at its padding width (#108).
    expect(base).not.toMatch(/width:\s*0/);
    expect(base).not.toMatch(/flex:/);
  });

  it("the row rule takes the space left of its neighbours in the two flex rows that size it", () => {
    const row = ruleBody(css, rowSelector);
    expect(row).toMatch(/flex:\s*1 1 auto/);
    expect(row).toMatch(/min-width:\s*0/);
    expect(row).toMatch(/max-width:\s*320px/);
  });

  it("the row rule lifts the cap on a phone, as it did before", () => {
    const phoneRow = ruleBody(ruleBody(css, "@media (max-width: 720px)"), rowSelector);
    expect(phoneRow).toMatch(/max-width:\s*none/);
  });

  it("the layer-row rule keeps renaming from moving the row", () => {
    // The field must keep the line box the name occupies: 15px at the body's 1.45
    // line-height, so the layer-text column and the row's height stay as they were.
    const rename = ruleBody(css, ".layer-text .name-field");
    expect(rename).toMatch(/height:\s*calc\(var\(--t15\) \* 1\.45\)/);
    expect(rename).not.toMatch(/flex:/);
  });
});
