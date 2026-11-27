import type { Metadata } from "next";
import { pageByUrl } from "../../../lib/docs-data.mjs";
import { docsMetadata } from "../../../lib/docs-meta";
import { ReadingPage } from "../../../components/docs/ReadingPage";

const page = pageByUrl.get("/docs/getting-started")!;
export const metadata: Metadata = docsMetadata(page);

export default function GettingStartedPage() {
  return (
    <ReadingPage
      page={page}
      breadcrumb={["Developer docs", "Guides", "API getting started"]}
      noteTitle="Loopback"
      note="The API listens on 127.0.0.1:9430. Its token is a bearer secret, so check the listener before you send it."
    />
  );
}
