import { DocsHeader } from "../../components/docs/DocsHeader";
import { Footer } from "../../components/Footer";
import "./docs.css";

export default function DocsLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className="docs-root">
      <DocsHeader />
      <main className="docs-main">{children}</main>
      <Footer />
    </div>
  );
}
