import { Info, Warning } from "@phosphor-icons/react/ssr";
import { parseInline, parseMarkdown, type Block, type InlineToken } from "../../lib/markdown.mjs";
import { CopyButton } from "../CopyButton";
import { breakable, HighlightedCode } from "./HighlightedCode";

function Inline({ tokens }: { tokens: InlineToken[] }) {
  return (
    <>
      {tokens.map((token, index) => {
        if (token.type === "text") return <span key={index}>{token.value}</span>;
        if (token.type === "code")
          return (
            <code key={index}>
              {breakable(token.value, `i${index}`)}
            </code>
          );
        if (token.type === "link")
          return (
            <a key={index} href={token.href}>
              <Inline tokens={token.children} />
            </a>
          );
        if (token.type === "strong")
          return (
            <strong key={index}>
              <Inline tokens={token.children} />
            </strong>
          );
        return (
          <em key={index}>
            <Inline tokens={token.children} />
          </em>
        );
      })}
    </>
  );
}

function Text({ value }: { value: string }) {
  return <Inline tokens={parseInline(value)} />;
}

/** Code comments take the muted ink; everything else stays in the page's own ink. */
function Code({ block }: { block: Extract<Block, { type: "code" }> }) {
  return (
    <div className="code-block">
      <div className="code-label">
        {block.lang || "text"} <span className="hair" />
      </div>
      <div className="code-well">
        <pre>
          <code>
            <HighlightedCode code={block.code} />
          </code>
        </pre>
        <div className="code-well-foot">
          <span>{block.lang || "text"}</span>
          <CopyButton text={block.code} />
        </div>
      </div>
    </div>
  );
}

function Quote({ text }: { text: string }) {
  const kind = /^\*\*Warning\*\*/i.test(text) ? "warning" : /^\*\*Tip\*\*/i.test(text) ? "tip" : "note";
  const label = /^\*\*(Warning|Tip|Note)\*\*:?/i.exec(text)?.[1] ?? "Note";
  const body = text.replace(/^\*\*(Warning|Tip|Note)\*\*:?\s*/i, "");
  return (
    <div className="callout" data-kind={kind}>
      {kind === "warning" ? (
        <Warning size={18} className="callout-icon" aria-hidden="true" />
      ) : (
        <Info size={18} className="callout-icon" aria-hidden="true" />
      )}
      <div>
        <div className="callout-label">{label}</div>
        <p>
          <Text value={body} />
        </p>
      </div>
    </div>
  );
}

/** The Markdown pages, rendered with the editor's tokens and the AC reading layout. */
export function MarkdownContent({ markdown, numbered = true }: { markdown: string; numbered?: boolean }) {
  const blocks = parseMarkdown(markdown);
  let section = 0;
  return (
    <div className="prose docs-prose">
      {blocks.map((block, index) => {
        if (block.type === "heading" && block.level === 1) {
          return null; // The page frame prints the title.
        }
        if (block.type === "heading" && block.level === 2) {
          section += 1;
          return (
            <h2 className="docs-h2" id={block.id} key={index}>
              {numbered ? <span className="seq">{String(section).padStart(2, "0")}</span> : null}
              <span>
                <Text value={block.text} />
              </span>
              <span className="hair" />
            </h2>
          );
        }
        if (block.type === "heading") {
          return (
            <h3 id={block.id} key={index}>
              <Text value={block.text} />
            </h3>
          );
        }
        if (block.type === "paragraph") {
          return (
            <p key={index}>
              <Text value={block.text} />
            </p>
          );
        }
        if (block.type === "list") {
          const Tag = block.ordered ? "ol" : "ul";
          return (
            <Tag key={index}>
              {block.items.map((item, at) => (
                <li key={at}>
                  <Text value={item} />
                </li>
              ))}
            </Tag>
          );
        }
        if (block.type === "code") return <Code block={block} key={index} />;
        if (block.type === "table") {
          return (
            <div className="docs-table-wrap" key={index}>
              <table className="a-param-table">
                <thead>
                  <tr>
                    {block.head.map((cell, at) => (
                      <th key={at}>
                        <Text value={cell} />
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {block.rows.map((row, at) => (
                    <tr key={at}>
                      {row.map((cell, cellAt) => (
                        <td key={cellAt}>
                          <Text value={cell} />
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          );
        }
        return <Quote text={block.text} key={index} />;
      })}
    </div>
  );
}
