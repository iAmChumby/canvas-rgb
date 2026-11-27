import Link from "next/link";
import { ArrowUpRight, BookOpen, Code, Fan, Shield, Terminal, Waves } from "@phosphor-icons/react/ssr";
import { sources } from "../../lib/docs-data.mjs";
import { parseMarkdown } from "../../lib/markdown.mjs";
import { withBase } from "../../lib/docs-meta";
import { PageActions } from "../../components/docs/PageActions";
import { DaemonBoard } from "../../components/docs/Board";
import { CodeWell } from "../../components/docs/CodeWell";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Developer docs / canvas-rgb docs",
  description: sources.home.summary,
  alternates: {
    canonical: withBase("/docs"),
    types: { "text/markdown": withBase("/docs.md") },
  },
};

const cards = [
  {
    href: "/docs/getting-started",
    icon: Shield,
    tone: "a1",
    title: "Connect a client",
    text: "Read the token, pass the Origin and Host checks, confirm who owns port 9430.",
  },
  {
    href: "/docs/api",
    icon: Code,
    tone: "a3",
    title: "Drive lights from the API",
    text: "Routes grouped by resource, with each route's body and answer types.",
  },
  {
    href: "/docs/effects",
    icon: BookOpen,
    tone: "a2",
    title: "Write an effect",
    text: "One HTML file. Tags, properties, globals and the draw loop.",
  },
  {
    href: "/docs/cli",
    icon: Terminal,
    tone: "a5",
    title: "Script the CLI",
    text: "Check a file before you install it, then import it without a window.",
  },
  {
    href: "/docs/api/fans",
    icon: Fan,
    tone: "a1",
    title: "Read the fans",
    text: "Headers, profiles and hot limits from the service.",
  },
  {
    href: "/docs/effects",
    icon: Waves,
    tone: "warn",
    title: "Build a library tile",
    text: "24 frames at 12.5 fps in 1.92 seconds, rendered headless and checked against the tile thresholds.",
  },
];

export default function DocsHome() {
  const blocks = parseMarkdown(sources.gettingStarted.body);
  const shellBlock = blocks.find((block) => block.type === "code" && block.lang === "sh") as
    | { code: string }
    | undefined;
  const shellCode = shellBlock?.code ?? "";
  return (
    <div className="c-wrap docs-home" data-page="home">
      <section className="c-hero">
        <div>
          <div className="c-kicker">
            <span className="led" /> Developer docs
          </div>
          <h1 className="c-title">Developer docs</h1>
          <p className="c-lead">{sources.home.summary}</p>
          <div className="c-actions">
            <Link className="btn btn-primary" href="/docs/api">
              <Code size={18} aria-hidden="true" /> API reference
            </Link>
            <Link className="btn" href="/docs/effects">
              <BookOpen size={18} aria-hidden="true" /> Write an effect
            </Link>
          </div>
          <PageActions mdPath="/docs.md" />
          <p className="agent-note">
            <strong>For agents.</strong> These docs are served as Markdown:{" "}
            <a href={withBase("/llms.txt")}>llms.txt</a> lists every page,{" "}
            <a className="token-nowrap" href={withBase("/llms-full.txt")}>llms-full.txt</a> holds them all in one file.
          </p>
        </div>
        <DaemonBoard />
      </section>

      <section className="c-section">
        <div className="c-section-head">
          <h2>Start from a task</h2>
          <p>Guides for each task.</p>
        </div>
        <div className="c-cards">
          {cards.map((card) => {
            const Icon = card.icon;
            return (
              <Link className="c-card" href={card.href} key={card.title}>
                <span className="c-card-icon" data-c={card.tone}>
                  <Icon size={18} aria-hidden="true" />
                </span>
                <b>{card.title}</b>
                <small>{card.text}</small>
                <ArrowUpRight size={16} className="icon" aria-hidden="true" />
              </Link>
            );
          })}
        </div>
      </section>

      <section className="c-section" id="token">
        <div className="c-section-head">
          <h2>Before the first request</h2>
          <p>The checks every client goes through.</p>
        </div>
        <div className="c-split">
          <div className="c-pane">
            <div className="c-pane-head">
              Terminal <span className="hair" /> <span className="mono">shell</span>
            </div>
            <div className="c-pane-body">
              <CodeWell code={shellCode} label="shell" />
            </div>
          </div>
          <div className="c-pane">
            <div className="c-pane-head">
              The four guards <span className="hair" />
            </div>
            <div className="c-pane-body">
              <div className="c-kv">
                <span>Bearer</span>
                <b>api-token</b>
              </div>
              <div className="c-kv">
                <span>Origin</span>
                <b>tauri.localhost</b>
              </div>
              <div className="c-kv">
                <span>Host</span>
                <b>127.0.0.1:9430</b>
              </div>
              <div className="c-kv">
                <span>Listener</span>
                <b>canvas-rgbd.exe</b>
              </div>
              <p>
                The token is 64 hex characters in <code>%APPDATA%\canvas-rgb\api-token</code>. Before you
                send it, check that port 9430 belongs to <code>canvas-rgbd.exe</code> running as your
                user. The <Link href="/docs/getting-started">getting-started page</Link> has the full
                check and the reason for each guard.
              </p>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
