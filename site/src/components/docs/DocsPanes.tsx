import Link from "next/link";
import { CheckCircle, LockKey, TerminalWindow } from "@phosphor-icons/react/ssr";
import {
  areaGroups,
  cli,
  jsSample,
  shapeFor,
  type CliCommand,
  type Field,
  type RouteEntry,
  type Shape,
} from "../../lib/docs-data.mjs";
import { parseInline, type Heading, type InlineToken } from "../../lib/markdown.mjs";
import { PageActions } from "./PageActions";
import { MarkdownContent } from "./MarkdownContent";
import { CodeWell, SampleCode } from "./CodeWell";
import { BreakableTypes, NoBreakText } from "./BreakableText";
import { SampleSwitch } from "./sample";

/** Meanings carry inline code spans. Render those as code rather than stripping their marks. */
function InlineTokens({ tokens }: { tokens: InlineToken[] }) {
  return (
    <>
      {tokens.map((token, index) => {
        if (token.type === "code") {
          return (
            <code key={index}>
              <NoBreakText value={token.value} />
            </code>
          );
        }
        if (token.type === "text") return <NoBreakText key={index} value={token.value} />;
        const children = <InlineTokens tokens={token.children ?? []} />;
        if (token.type === "link")
          return (
            <a key={index} href={token.href}>
              {children}
            </a>
          );
        if (token.type === "strong") return <strong key={index}>{children}</strong>;
        return <em key={index}>{children}</em>;
      })}
    </>
  );
}

/** A reader-facing sentence from the reference, with its inline code intact. */
export function InlineText({ value }: { value?: string }) {
  return <InlineTokens tokens={parseInline(value ?? "")} />;
}

export function SectionHead({ number, title, id }: { number: number; title: string; id?: string }) {
  return (
    <h2 className="a-section-head" id={id}>
      <span className="seq">{String(number).padStart(2, "0")}</span>
      <span>{title}</span>
      <span className="hair" />
    </h2>
  );
}

/** Field tables keep the third column only where the content describes what a field holds. */
export function FieldTable({ shape }: { shape: Shape }) {
  if (!shape.fields.length) return null; // The shape card already prints the note.
  const explained = shape.fields.some((field: Field) => field.meaning);
  return (
    <table className="a-param-table">
      <thead>
        <tr>
          <th>Field</th>
          <th>Type</th>
          <th>{explained ? "What it holds" : "Required"}</th>
        </tr>
      </thead>
      <tbody>
        {shape.fields.map((field: Field) => (
          <tr key={field.name}>
            <td><NoBreakText value={field.name} /></td>
            <td className="doc-type"><BreakableTypes value={field.type} /></td>
            <td>
              {explained ? (
                <>
                  <InlineText value={field.meaning} />
                  {field.optional ? <span className="field-optional">Optional</span> : null}
                </>
              ) : field.optional ? (
                "Optional"
              ) : (
                "Required"
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** A string enum publishes one meaning per value; keep them beside the fields that use it. */
function EnumValues({ shape }: { shape: Shape }) {
  if (!shape.values?.length) return null;
  return (
    <div className="docs-fields">
      <div className="a-shape-title">Values</div>
      <table className="a-param-table">
        <thead>
          <tr>
            <th>Value</th>
            <th>What it means</th>
          </tr>
        </thead>
        <tbody>
          {shape.values.map((value) => (
            <tr key={value.value}>
              <td>
                <code><NoBreakText value={value.value} /></code>
              </td>
              <td>
                <InlineText value={value.meaning} />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function ShapeCard({ title, shape, tone }: { title: string; shape: Shape; tone: string }) {
  return (
    <div className="a-shape">
      <div className="a-shape-title">
        {title} <span className="led" data-on={tone} />
      </div>
      <div className="a-shape-type"><BreakableTypes value={shape.label} /></div>
      {shape.note ? (
        <p>
          <InlineText value={shape.note} />
        </p>
      ) : null}
    </div>
  );
}

/** A's four fixed sections, filled from the published reference. */
export function RouteContent({ entry }: { entry: RouteEntry }) {
  const group = areaGroups.find((area) => area.area === entry.area);
  return (
    <>
      <section className="a-section">
        <SectionHead number={1} title="Request" id="request" />
        {entry.methods.map((method) => (
          <div className="docs-method-block" key={method}>
            {entry.methods.length > 1 ? (
              <div className="docs-method-line">
                <span className="method" data-m={method}>
                  {method}
                </span>
              </div>
            ) : null}
            <div className="code-block">
              <div className="code-label">
                Shell <span className="hair" />
              </div>
              <SampleCode
                scope={entry.slug}
                initial={Object.values(entry.responseType).includes("WebSocket (binary frames)") ? "js" : "curl"}
                samples={{ curl: entry.examples[method] ?? "", js: jsSample(entry, method) }}
              />
            </div>
          </div>
        ))}
      </section>

      <section className="a-section">
        <SectionHead number={2} title="Types" id="types" />
        {entry.methods.map((method) => {
          const request = shapeFor(entry.requestType[method], entry.requestShape?.[method]);
          const answer = shapeFor(entry.responseType[method], entry.responseShape?.[method]);
          const params = entry.queryParams?.[method] ?? [];
          return (
            <div className="docs-method-block" key={method}>
              {entry.methods.length > 1 ? (
                <div className="docs-method-line">
                  <span className="method" data-m={method}>
                    {method}
                  </span>
                </div>
              ) : null}
              <div className="a-two">
                <ShapeCard title="Request body" shape={request} tone="" />
                <ShapeCard title="Answer body" shape={answer} tone="a1" />
              </div>
              {request.fields.length ? (
                <div className="docs-fields">
                  <div className="a-shape-title">Request fields</div>
                  <FieldTable shape={request} />
                </div>
              ) : null}
              {answer.fields.length ? (
                <div className="docs-fields">
                  <div className="a-shape-title">Answer fields</div>
                  <FieldTable shape={answer} />
                </div>
              ) : null}
              <EnumValues shape={request} />
              <EnumValues shape={answer} />
              {params.length ? (
                <div className="docs-params">
                  <div className="a-shape-title">Query parameters</div>
                  <table className="a-param-table">
                    <thead>
                      <tr>
                        <th>Query</th>
                        <th>Type</th>
                        <th>What it holds</th>
                      </tr>
                    </thead>
                    <tbody>
                      {params.map((param) => (
                        <tr key={param.name}>
                          <td><NoBreakText value={param.name} /></td>
                          <td className="doc-type"><BreakableTypes value={param.type} /></td>
                          <td>
                            <InlineText value={param.meaning} />
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ) : null}
            </div>
          );
        })}
        {entry.pathParams.length ? (
          <div className="docs-params">
            <div className="a-shape-title">Path parameters</div>
            <table className="a-param-table">
              <thead>
                <tr>
                  <th>Path</th>
                  <th>Type</th>
                  <th>What it holds</th>
                </tr>
              </thead>
              <tbody>
                {entry.pathParams.map((param) => (
                  <tr key={param.name}>
                    <td>
                      <code><NoBreakText value={param.name} /></code>
                    </td>
                    <td className="doc-type"><BreakableTypes value={param.type} /></td>
                    <td>
                      <InlineText value={param.meaning} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </section>

      <section className="a-section">
        <SectionHead number={3} title="Route index" id="route-index" />
        <p className="prose docs-note">Every route in {entry.area}.</p>
        <div className="docs-table-wrap">
          <table className="a-param-table a-stack-table">
            <thead>
              <tr>
                <th>Method / path</th>
                <th>Body</th>
                <th>Answer</th>
              </tr>
            </thead>
            <tbody>
              {group?.routes.flatMap((route) =>
                route.methods.map((method) => (
                  <tr key={`${route.slug}-${method}`}>
                    <td data-label="Method / path">
                      <Link href={`/docs/api/${route.slug}`}>
                        <NoBreakText value={method} />{" "}
                        <NoBreakText value={route.path} />
                      </Link>
                    </td>
                    <td data-label="Body">
                      <BreakableTypes value={route.requestType[method] ?? ""} />
                    </td>
                    <td data-label="Answer">
                      <BreakableTypes value={route.responseType[method] ?? ""} />
                    </td>
                  </tr>
                )),
              )}
            </tbody>
          </table>
        </div>
      </section>

      <section className="a-section">
        <SectionHead number={4} title="Example" id="example" />
        {entry.methods.map((method) => {
          const body = /-d '([^']+)'/.exec(entry.examples[method] ?? "")?.[1];
          return (
            <div className="docs-example" key={method}>
              <p className="prose">
                <span className="method" data-m={method}>
                  {method}
                </span>{" "}
                <InlineText value={entry.descriptions[method]} />
              </p>
              {body ? (
                <div className="code-block">
                  <div className="code-label">
                    json <span className="hair" />
                  </div>
                  <CodeWell
                    code={JSON.stringify(JSON.parse(body), null, 2)}
                    label="request body"
                  />
                </div>
              ) : (
                <p className="prose docs-note">This route takes no request body.</p>
              )}
            </div>
          );
        })}
      </section>
    </>
  );
}

/** The CLI command page, in A's frame with the command's own flags and exit codes. */
export function CommandContent({ command }: { command: CliCommand }) {
  const json = JSON.stringify(
    { command: command.syntax, flags: command.flags, exitCodes: command.exitCodes.map((c) => `${c.code}: ${c.meaning}`) },
    null,
    2,
  );
  return (
    <>
      <section className="a-section">
        <SectionHead number={1} title="Syntax" id="syntax" />
        <div className="code-block">
          <div className="code-label">
            Terminal <span className="hair" />
          </div>
          <SampleCode scope={command.slug} initial="shell" samples={{ shell: command.syntax, json }} />
        </div>
        {command.flags.some((flag) => flag.flag === "--from")
          ? command.sectionNotes?.map((note, at) => (
              <div className="docs-example" key={at}>
                <div className="a-shape-title">{note.title}</div>
                <MarkdownContent markdown={note.lines.join("\n")} numbered={false} />
              </div>
            ))
          : null}
      </section>

      <section className="a-section">
        <SectionHead number={2} title="Flags" id="flags" />
        {command.flags.length ? (
          <table className="a-param-table a-flags-table">
            <thead>
              <tr>
                <th>Flag</th>
                <th>VALUE</th>
                <th>What it does</th>
              </tr>
            </thead>
            <tbody>
              {command.flags.map((flag) => (
                <tr key={flag.flag}>
                  <td>
                    <code>{flag.flag}</code>
                  </td>
                  <td className="a-flag-value">
                    <code>{flag.value ?? "—"}</code>
                  </td>
                  <td>
                    <InlineText value={flag.meaning} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="prose docs-note">No command-specific flags. Global flags still apply.</p>
        )}
      </section>

      <section className="a-section">
        <SectionHead number={3} title="Exit codes" id="exit-codes" />
        <table className="a-param-table">
          <thead>
            <tr>
              <th>Code</th>
              <th>Meaning</th>
            </tr>
          </thead>
          <tbody>
            {command.exitCodes.map((code) => (
              <tr key={code.code}>
                <td>{code.code}</td>
                <td>
                  <InlineText value={code.meaning} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="a-section">
        <SectionHead number={4} title="Global flags" id="global-flags" />
        <table className="a-param-table a-flags-table">
          <thead>
            <tr>
              <th>Flag</th>
              <th>VALUE</th>
              <th>What it does</th>
            </tr>
          </thead>
          <tbody>
            {cli.globalFlags.map((flag) => (
              <tr key={flag.flag}>
                <td>
                  <code>{flag.flag}</code>
                </td>
                <td className="a-flag-value">
                  <code>{flag.value ?? "—"}</code>
                </td>
                <td>
                  <InlineText value={flag.what} />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </>
  );
}

/** The sticky right pane: actions, outline, the code switch and an auth note. */
export function DocsAside({
  mdPath,
  headings: outline,
  scope,
  sampleLabels,
  noteTitle,
  note,
}: {
  mdPath: string;
  headings: Heading[];
  scope?: string;
  sampleLabels?: Array<{ value: string; label: string }>;
  noteTitle?: string;
  note?: string;
}) {
  return (
    <aside className="a-aside" aria-label="On this page">
      <PageActions mdPath={mdPath} />
      <div className="a-aside-title">On this page</div>
      <ul className="a-toc">
        {outline.map((heading, index) => (
          <li key={`${heading.id}-${index}`}>
            <a href={`#${heading.id}`} className={index === 0 ? "is-active" : undefined}>
              {heading.text}
            </a>
          </li>
        ))}
      </ul>
      {scope && sampleLabels ? (
        <>
          <div className="a-aside-title docs-aside-title">Code sample</div>
          <SampleSwitch scope={scope} labels={sampleLabels} value={sampleLabels[0]?.value ?? "curl"} />
        </>
      ) : null}
      {note ? (
        <div className="a-aside-callout">
          <div className="eyebrow">{noteTitle}</div>
          <p>{note}</p>
        </div>
      ) : null}
    </aside>
  );
}

/** The small badges under a route or command heading. */
export function RouteBadges({ badges }: { badges: string[] }) {
  return (
    <span className="a-route-badges">
      {badges.map((badge) => (
        <span className="a-badge" key={badge}>
          {badge === "No daemon" ? <CheckCircle size={14} aria-hidden="true" /> : null}
          {badge === "No token" ? <LockKey size={14} aria-hidden="true" /> : null}
          {badge === "Local" ? <TerminalWindow size={14} aria-hidden="true" /> : null}
          {badge}
        </span>
      ))}
    </span>
  );
}
