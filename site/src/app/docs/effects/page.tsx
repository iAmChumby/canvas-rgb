import type { Metadata } from "next";
import { pageByUrl } from "../../../lib/docs-data.mjs";
import { docsMetadata } from "../../../lib/docs-meta";
import { ReadingPage } from "../../../components/docs/ReadingPage";
import { EffectBoard } from "../../../components/docs/Board";

const page = pageByUrl.get("/docs/effects")!;
export const metadata: Metadata = docsMetadata(page);

export default function EffectsPage() {
  return (
    <ReadingPage
      page={page}
      breadcrumb={["Developer docs", "Guides", "Writing an effect"]}
      board={<EffectBoard />}
      noteTitle="Runtime"
      note="The runtime uses QuickJS and a Canvas 2D context. Inline scripts run in order."
    />
  );
}
