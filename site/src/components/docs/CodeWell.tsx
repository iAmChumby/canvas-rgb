"use client";

import { CopyButton } from "../CopyButton";
import { HighlightedCode } from "./HighlightedCode";
import { useSample } from "./sample";

/** A recessed code sample with its printed label. */
export function CodeWell({ code, label }: { code: string; label: string }) {
  return (
    <div className="code-well">
      <pre>
        <code>
          <HighlightedCode code={code} />
        </code>
      </pre>
      <div className="code-well-foot">
        <span>{label}</span>
        <CopyButton text={code} />
      </div>
    </div>
  );
}

/** The sample the right pane's switch selected. */
export function SampleCode({
  scope,
  samples,
  initial,
}: {
  scope: string;
  samples: Record<string, string>;
  initial: string;
}) {
  const keys = Object.keys(samples);
  const value = useSample(scope, initial, keys);
  return <CodeWell code={samples[value] ?? samples[initial] ?? ""} label={value} />;
}
